//! Calls (`spec/04 §3`): labeled arguments and punning (U2004 to U2006), calls of functions,
//! function values, policies and capability operations, field access and method-call syntax
//!, and constructors (U2009).

use una_diag::{Code, ObligationKind};
use una_syntax::Span;
use una_syntax::ast::{Arg, Expr, ExprKind, Ident};

use super::Report;
use super::check::{Checker, Role, Site};
use super::closest;
use super::env::{Entity, FieldDef, FnDef, FnKind, ParamDef};
use super::expr::count;
use super::label::Label;
use super::records::Res;
use super::ty::{FnTy, FnUse, KindId, Refinement, Ty};
use super::unify::Vars;

/// A checked argument: where it is, its type, the type of its parameter, and the parameter.
pub(crate) type Checked<'a> = (Span, Ty, Ty, &'a str);

impl<'a> Checker<'_, 'a, '_> {
    /// `e.f`: `.len` of `Text` and `List`, a field of `e`'s record, or a call `f(e)` of a
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
        if let Some(ty) = self.record_field(labeled.unlabeled(), field) {
            if let Some((kind, index)) = self.field_index(&resolved, field) {
                self.resolved(span, Res::Field { kind, index });
            }
            return ty.with_label(&top);
        }
        match resolved {
            Ty::Error | Ty::Never => return Ty::Error,
            Ty::Var(_) => {
                self.unknown_type(target.span);
                return Ty::Error;
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
                Ty::Error
            }
            _ => {
                self.unknown_member(&resolved, name, false);
                Ty::Error
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
        self.report.push(diagnostic);
    }

    /// `e.f(args)`: a call of field `f` when it has a function type, otherwise `f(e, args)` with
    /// `f` declared in scope (`spec/04 §3`).
    pub(crate) fn method(
        &mut self,
        target: &Expr,
        name: &Ident,
        args: &[Arg],
        span: Span,
        site: Site,
    ) -> Ty {
        let target_ty = self.expr(target, None);
        let method = self.name(name);
        let resolved = self.resolve(&target_ty);
        let labeled = self.labeled(&target_ty);
        if let Some(field_ty) = self
            .record_field(labeled.unlabeled(), method)
            .map(|ty| ty.with_label(&labeled.top_label()))
            && matches!(self.resolve(&field_ty), Ty::Fn(_))
        {
            self.outside_rl(span, || format!("call of field `{method}`"));
            let ty = self.call_value(&field_ty, args, span, &format!("field `{method}`"), site);
            if let Some((kind, index)) = self.field_index(&resolved, method) {
                self.resolved(span, Res::FieldFn { kind, index });
            }
            return ty;
        }
        match self.env.lower(self.module, method) {
            Some(Entity::Function(id)) => {
                self.call_function(id, Some((target.span, target_ty)), args, span, site)
            }
            _ => {
                match resolved {
                    Ty::Error | Ty::Never => {}
                    Ty::Var(_) => self.unknown_type(target.span),
                    _ => self.unknown_member(&resolved, name, true),
                }
                self.unchecked(args);
                Ty::Error
            }
        }
    }

    /// Checks arguments that belong to no parameter, for the diagnostics inside them.
    fn unchecked(&mut self, args: &[Arg]) {
        for arg in args {
            self.expr(&arg.value, None);
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
                    return self.call_value(&ty, args, span, &format!("`{name}`"), site);
                }
                match self.env.lower(self.module, name) {
                    Some(Entity::Function(id)) => self.call_function(id, None, args, span, site),
                    Some(Entity::Handler(_)) => {
                        self.report.error(
                            Code::U2001,
                            ident.span,
                            format!(
                                "handler `{name}` is not a function; install it with `with` or `given`"
                            ),
                        );
                        self.unchecked(args);
                        Ty::Error
                    }
                    // An unknown name is U1001, reported by M3.
                    _ => {
                        self.unchecked(args);
                        Ty::Error
                    }
                }
            }
            ExprKind::UpperName(ident) => self.constructor_call(ident, args, span, expected),
            _ => {
                self.outside_rl(span, || "call of a function value".to_owned());
                let ty = self.expr(callee, None);
                self.call_value(&ty, args, span, "this function", site)
            }
        }
    }

    /// A call of a function value: positional arguments only.
    fn call_value(&mut self, ty: &Ty, args: &[Arg], span: Span, what: &str, site: Site) -> Ty {
        let value_label = self.labeled(ty).top_label();
        // The function type is taken before its labels are settled: the result of a generic
        // function value carries the labels of type arguments that the arguments of this call
        // may still solve (ADR 0038 D3.2, DECISION D-137, re-review N-003).
        let head = self.vars.head(ty);
        let function = match head.unlabeled().clone() {
            Ty::Fn(function) => *function,
            Ty::Labeled(..) => {
                self.unchecked(args);
                return Ty::Error;
            }
            Ty::Error | Ty::Never => {
                self.unchecked(args);
                return Ty::Error;
            }
            Ty::Var(_) => {
                self.unknown_type(span);
                self.unchecked(args);
                return Ty::Error;
            }
            other => {
                let shown = self.show(&other);
                self.report.error(
                    Code::U2001,
                    span,
                    format!("{what} is not a function; it has type `{shown}`"),
                );
                self.unchecked(args);
                return Ty::Error;
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
            // An argument decided by comparing values of the enclosing function's type
            // parameters carries `ℓ*_T`, which its flow into `T` reads as `ℓ_T`, the label the
            // function value is checked against at each call. The decision raises the call's
            // result and program counter instead, as the value's own label does (#253).
            decided = decided.join(&self.decisions(&found));
            flows.push((arg.value.span, found, param.clone()));
        }
        for (arg_span, found, param) in flows {
            self.instantiated_by(arg_span, &param, &found);
            self.flow(arg_span, &found, &param, &format!("the argument of {what}"));
        }
        let value_label = value_label.join(&decided);
        // Calling a function value needs the capabilities of its type (`spec/10 §2`, S-Fun).
        let level = self.frames.len();
        self.needs_site(what, &function.uses, span, level);
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
        self.position_site(
            what,
            span,
            !function.uses.is_empty(),
            function.fails.as_ref(),
            site.top,
            site.handled,
        );
        if let Some(fails) = &function.fails {
            self.call_fails.insert(span, fails.clone());
        }
        // The result of a function value carries the value's label.
        function.result.with_label(&value_label)
    }

    /// #253, #350: a call that gives function values data decided by values of
    /// the enclosing function's type parameters. The arguments carry `ℓ*_T`; a function value
    /// whose parameters lack it was checked against `ℓ_T` at each call, as a named function
    /// value or a lambda whose parameter is annotated (the body assumes the annotation) is.
    /// Whoever calls it with that data, the prelude or a generic function, may reveal the
    /// decision through what it returns, through its effects and through its failures, its
    /// own or the function value's. So the result is raised at every position, the callee's
    /// effects and the function values' run under the raised program counter (T-Call, T-App,
    /// as #253 does for a call written in the body), and its failure is labeled by it, as
    /// N-39 labels a host operation's. A lambda whose parameter types are inferred takes
    /// `ℓ*_T` in them, and its own flows carry it. Returns the label of the decision.
    fn decided_callbacks(
        &mut self,
        callee: &str,
        span: Span,
        needs: &[FnUse],
        fails: bool,
        checked: &[Checked<'a>],
    ) -> Label {
        let mut raised = Label::public();
        if self.type_params.is_empty() {
            return raised;
        }
        // Every decision the arguments carry, at any position, beside callbacks too.
        let mut decided = Label::public();
        for (_, found, _, _) in checked {
            decided = decided.join(&self.decisions(found));
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
                    let mut every: Option<std::collections::BTreeSet<String>> = None;
                    for input in &function.params {
                        let carried = self.decisions(input);
                        every = Some(match every {
                            None => carried.symbols,
                            Some(so_far) => {
                                so_far.intersection(&carried.symbols).cloned().collect()
                            }
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
            Ty::List(element) => self.function_types_in(element, field, out, seen),
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

    /// The deep symbolic labels `ℓ*_T` of the enclosing function's type parameters at any
    /// position of `found` (#253).
    fn decisions(&self, found: &Ty) -> Label {
        if self.type_params.is_empty() {
            return Label::public();
        }
        let names = self.type_params.clone();
        let seen = std::cell::RefCell::new(Label::public());
        self.labeled(found).map_labels(&|label| {
            let mut gathered = Label::public();
            gathered.symbols = label
                .symbols
                .iter()
                .filter(|s| {
                    s.strip_suffix('*')
                        .is_some_and(|name| names.contains(&name))
                })
                .cloned()
                .collect();
            let joined = seen.borrow().join(&gathered);
            *seen.borrow_mut() = joined;
            label.clone()
        });
        seen.into_inner()
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
            return Ty::Error;
        };
        let callee = match def.kind {
            FnKind::Function => format!("`{}`", def.name),
            FnKind::Predicate { .. } => format!("predicate `{}`", def.name),
            FnKind::Policy => format!("policy `{}`", def.name),
        };
        if !matches!(def.kind, FnKind::Predicate { .. }) {
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
        // opaque bodies. DECISION D-149: also in the `where` clause of a kind and in the
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
            diagnostic.repairs.push(super::Report::advice(format!(
                "check the value with `checked(value, {}(it)) else fail <Variant>`",
                def.name
            )));
            self.report.push(diagnostic);
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
        self.testing_site(def.module, def.name, span);
        let needs: Vec<FnUse> = def.uses.iter().map(|need| need.usage.clone()).collect();
        let level = self.frames.len();
        self.needs_site(&callee, &needs, span, level);
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
        self.position_site(
            &callee,
            span,
            !needs.is_empty(),
            def.fails.as_ref(),
            site.top,
            site.handled,
        );
        if def.kind == FnKind::Policy {
            self.resolved(
                span,
                Res::Function {
                    id,
                    type_args: Vec::new(),
                },
            );
            return self.call_policy(&def, &callee, receiver, args, span);
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
        let checked = self.arguments(
            &callee,
            &def.params,
            &subst,
            receiver,
            args,
            span,
            def.module,
        );
        let values = declared_args(&def.params, &checked);
        let result = def.result.base.substitute(&subst);
        let decided = self.decided_callbacks(&callee, span, &needs, def.fails.is_some(), &checked);
        let result = if prelude_labeled(self.env.prelude, &def) {
            self.prelude_result(&callee, def.name, checked, result)
        } else {
            self.argument_flows(&callee, checked);
            generic_result(self.env.prelude, &def, &subst, result)
        };
        if !subst.is_empty() {
            self.instantiation(def.name, &def.type_params, subst, values, span);
        }
        if def.requires {
            self.obligation(ObligationKind::Precondition, span, None);
        }
        let result = if decided.is_public() {
            result
        } else {
            result.raise_deep(&decided)
        };
        if let Some(fails) = &def.fails {
            // Whether the call fails, and how, may reveal a decision given to its function
            // values, as an operation's failure reveals its arguments (N-39, #350).
            if decided.is_public() {
                self.call_fails.insert(span, fails.clone());
            } else {
                self.call_fails.insert(span, fails.raise_deep(&decided));
                self.decided_failures.insert(span, decided);
            }
        }
        result
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
    ///   element types included (ADR 0038 D3.4).
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
        result.with_label(&label)
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
        def: &FnDef<'a>,
        callee: &str,
        receiver: Option<(Span, Ty)>,
        args: &[Arg],
        span: Span,
    ) -> Ty {
        let from = def.params.first().map(|p| p.ty.clone());
        let expected = from.as_ref().map_or(Ty::Error, |from| from.base.clone());
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
            self.call_fails.insert(span, fails.clone());
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
        let capability = self
            .op_site(cap, op)
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
            self.unchecked(args);
            return Ty::Error;
        };
        let Some(def) = capability.ops.iter().find(|o| o.name == op_name).cloned() else {
            let names: Vec<&str> = capability.ops.iter().map(|o| o.name).collect();
            let mut diagnostic = self.report.make(
                Code::U1001,
                op.span,
                format!(
                    "capability `{}` has no operation `{op_name}`",
                    capability.name
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
            return Ty::Error;
        };
        if let Some((capability_id, _)) = self.op_index(cap_name, op_name) {
            self.resolved(
                span,
                Res::Op {
                    capability: capability_id,
                },
            );
        }
        let callee = format!("operation `{}.{op_name}`", capability.name);
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
        // and how (N-39).
        let mut decided = Label::public();
        let host = capability.module == self.env.prelude;
        let mut sinks: Vec<(Span, Ty)> = Vec::new();
        for (arg_span, found, expected, param) in checked {
            if host && decides_failure(capability.name, op_name, param) {
                // Capped at what the operation accepts: an argument above it is U4003 already,
                // so `Keys.hmac` and `Files.read`, which accept only `public`, are unchanged.
                let accepted = def.accepts.clone().unwrap_or_else(Label::public);
                decided = decided.join(&self.deep(&found).meet(&accepted));
            }
            // ADR 0050: the argument flows into the declared parameter type at its
            // contravariant positions and invariant kind arguments, which a handler body
            // assumes; `accepts` bounds its data.
            self.op_argument_flow(
                arg_span,
                &found,
                &expected,
                &format!("parameter `{param}` of {callee}"),
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
        if let Some(fails) = &def.fails {
            // Whether a host operation fails, and how, is labeled like the arguments that
            // decide it (`spec/07 §7` rule 4, N-39): its payload too, and a handling clause
            // branches on that label.
            if decided.is_public() {
                self.call_fails.insert(span, fails.clone());
            } else {
                self.call_fails.insert(span, fails.raise_deep(&decided));
                self.decided_failures.insert(span, decided);
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
        def.result.base.with_label(&carried)
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
    fn punned(&self, arg: &Arg, names: &[&str]) -> Option<usize> {
        if arg.label.is_some() {
            return None;
        }
        let ExprKind::Name(ident) = &arg.value.kind else {
            return None;
        };
        let name = self.name(ident);
        names.iter().position(|candidate| *candidate == name)
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
        let names: Vec<&str> = params.iter().map(|p| p.name).collect();
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
        if let (Some((receiver_span, receiver_ty)), Some(first)) = (&receiver, params.first()) {
            let expected = first.ty.base.substitute(subst);
            let role = Role::Argument {
                param: first.name,
                callee,
            };
            self.unify_at(*receiver_span, &expected, receiver_ty, role);
            self.fn_flow(*receiver_span, &expected, receiver_ty);
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
            let target = if let Some(label) = &arg.label {
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
            let role = Role::Argument {
                param: param.name,
                callee,
            };
            self.ctx.argument = true;
            let found = self.expr_as(&arg.value, Some(&expected), role);
            self.fn_flow(arg.value.span, &expected, &found);
            if param.ty.is_refined() {
                let origin = self.origin(module, param.span);
                self.obligation(ObligationKind::Refinement, arg.value.span, origin);
            }
            checked.push((arg.value.span, found, expected, param.name));
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
    fn unknown_label(&mut self, callee: &str, names: &[&str], label: &Ident) {
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
    fn missing_label(&mut self, callee: &str, names: &[&str], arg: &Arg, position: usize) {
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
        if self.type_params.contains(&name) {
            self.report.error(
                Code::U2001,
                ident.span,
                format!("type parameter `{name}` is not a constructor"),
            );
            self.unchecked(args);
            return Ty::Error;
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
                self.construct(kind, Some(index), &targs, args, ident.span, span);
                return Ty::Kind(kind, targs);
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
                self.construct(id, None, &targs, args, ident.span, span);
                return Ty::Kind(id, targs);
            }
            Some(Entity::Kind(_)) => {
                format!("`{name}` is a sum kind; construct one of its variants")
            }
            Some(Entity::Alias(_) | Entity::Builtin(_)) => {
                format!("`{name}` is a type, not a constructor")
            }
            Some(Entity::Capability(_)) => format!("capability `{name}` is not a value"),
            Some(Entity::Function(_) | Entity::Handler(_)) | None => {
                self.unchecked(args);
                return Ty::Error;
            }
        };
        self.report.error(Code::U2001, ident.span, problem);
        self.unchecked(args);
        Ty::Error
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

    /// A constructor application, also after `fail` (`spec/04 §3` rule 4): with two or more
    /// fields every argument is labeled or punned, in any order, and one field may go without
    /// a label. Missing and unknown fields are U2009, a repeated field U2006, and an unlabeled
    /// argument where labels are required U2004. `span` covers the name and
    /// the arguments, `name` only the name.
    pub(crate) fn construct(
        &mut self,
        kind: KindId,
        index: Option<usize>,
        targs: &[Ty],
        args: &[Arg],
        name: Span,
        span: Span,
    ) {
        let Some(def) = self.env.kinds.get(kind) else {
            return;
        };
        let module = def.module;
        let (fields, constructor): (Vec<FieldDef<'a>>, &'a str) = match index {
            Some(index) => match def.variants.get(index) {
                Some(variant) => (variant.fields.clone(), variant.name),
                None => return,
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
            return;
        }
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        let mut taken: Vec<bool> = fields.iter().map(|_| false).collect();
        // R-030: the repair of an unknown field suggests only a field that no other argument
        // gives, so applying it cannot make the argument a duplicate (U2006; DECISION D-157).
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
                        // DECISION D-157, as for U2005 (re-review N-016).
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
            let found = self.expr_as(&arg.value, Some(&expected), role);
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
        // Fields flow once the whole constructor is typed.
        for (arg_span, found, expected, field) in flows {
            self.flow(
                arg_span,
                &found,
                &expected,
                &format!("field `{field}` of `{constructor}`"),
            );
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
    }
}

/// The prelude functions whose results carry the labels of their arguments, as `spec/14 §1`
/// "Result labels" lists them (ADR 0038 D3.4): their native implementations declare these
/// labels, which the prelude's Una signatures cannot write.
const PRELUDE_LABELED: [&str; 20] = [
    "abs",
    "append",
    "at",
    "contains",
    "filter",
    "first",
    "first_where",
    "fold",
    "is_digits",
    "is_empty",
    "is_ok",
    "is_some",
    "map",
    "max",
    "min",
    "or_else",
    "starts_with",
    "sum",
    "to_int",
    "trim",
];

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
        _ => return None,
    })
}

/// Whether the parameter `param` of the host operation `capability.op` decides whether it
/// fails, and how (`spec/07 §7` rule 4, N-39): the path of `Files`, not the content; the key
/// name of `Keys`, not the data.
fn decides_failure(capability: &str, op: &str, param: &str) -> bool {
    match (capability, op) {
        ("Files", _) => param == "path",
        ("Keys", "hmac") => param == "key_name",
        _ => false,
    }
}

/// Whether `def` is a prelude function whose result carries the labels of its arguments.
/// DECISION D-136: its parameters that are not functions accept arguments of any label,
/// since the result carries all of them, at the positions that hold them for the structural
/// functions (ADR 0051); the other prelude functions keep the labels of their parameter types.
fn prelude_labeled(prelude: usize, def: &FnDef<'_>) -> bool {
    def.module == prelude && def.kind == FnKind::Function && PRELUDE_LABELED.contains(&def.name)
}

/// The result of a call of a generic function declared outside the prelude: the value its
/// body returns may carry the symbolic labels of its type parameters, which the call replaces
/// by `ℓ*(σ)`, the join of every label of each type argument (`spec/10 §5`, generic
/// instantiation, rule 3).
/// DECISION D-137: every position of the result that holds data is raised by the join of
/// `ℓ*(σ)` over all the type arguments, since the checker does not record which symbolic
/// labels, at which positions, the body's result carries.
fn generic_result(prelude: usize, def: &FnDef<'_>, subst: &[Ty], result: Ty) -> Ty {
    if subst.is_empty() || def.module == prelude {
        return result;
    }
    result.raise_deep(&Vars::label_of(subst))
}

/// Whether `ty` holds a type variable at any position.
fn holds_var(ty: &Ty) -> bool {
    match ty {
        Ty::Var(_) => true,
        Ty::Labeled(_, inner) | Ty::List(inner) => holds_var(inner),
        Ty::Kind(_, args) => args.iter().any(holds_var),
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
                && *kind != Ty::Error
                && e.fails
                    .as_ref()
                    .is_none_or(|allowed| allowed != kind && *allowed != Ty::Error)
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
        (Ty::List(e), Ty::List(f)) => fn_problem(e, f, flipped),
        (Ty::Kind(i, es), Ty::Kind(j, fs)) if i == j => es
            .iter()
            .zip(fs)
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
        (Ty::List(e), Ty::List(f)) => refinement_flow(e, f),
        (Ty::Kind(i, es), Ty::Kind(j, fs)) if i == j && es.len() == fs.len() => {
            es.iter().zip(fs).any(|(e, f)| refinement_flow(e, f))
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
