//! Calls (`spec/04 §3`): labeled arguments and punning (U2004 to U2006), calls of functions,
//! function values, policies and capability operations, field access and method-call syntax
//! (DECISION D-052), and constructors (U2009).

use una_diag::{Code, ObligationKind};
use una_syntax::Span;
use una_syntax::ast::{Arg, Expr, ExprKind, Ident};

use super::Report;
use super::check::{Checker, Role};
use super::closest;
use super::env::{Entity, FieldDef, FnDef, FnKind, ParamDef};
use super::expr::count;
use super::ty::{KindId, Ty};

impl<'a> Checker<'_, 'a, '_> {
    /// `e.f`: `.len` of `Text` and `List`, a field of `e`'s record, or a call `f(e)` of a
    /// function of one parameter declared in scope (`spec/04 §3`, DECISION D-052); U1001 when
    /// none applies.
    pub(crate) fn field(&mut self, target: &Expr, name: &Ident, span: Span) -> Ty {
        let target_ty = self.expr(target, None);
        let field = self.name(name);
        let resolved = self.resolve(&target_ty);
        if field == "len" && matches!(resolved, Ty::Text | Ty::List(_)) {
            return Ty::Int;
        }
        if let Some(ty) = self.record_field(&resolved, field) {
            return ty;
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
                    return self.call_function(
                        id,
                        Some((target.span, target_ty)),
                        &[],
                        span,
                        false,
                    );
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
    /// `f` declared in scope (`spec/04 §3`, DECISION D-052).
    pub(crate) fn method(
        &mut self,
        target: &Expr,
        name: &Ident,
        args: &[Arg],
        span: Span,
        handled: bool,
    ) -> Ty {
        let target_ty = self.expr(target, None);
        let method = self.name(name);
        let resolved = self.resolve(&target_ty);
        if let Some(field_ty) = self.record_field(&resolved, method)
            && matches!(self.resolve(&field_ty), Ty::Fn(_))
        {
            self.outside_rl(span, || format!("call of field `{method}`"));
            return self.call_value(&field_ty, args, span, &format!("field `{method}`"));
        }
        match self.env.lower(self.module, method) {
            Some(Entity::Function(id)) => {
                self.call_function(id, Some((target.span, target_ty)), args, span, handled)
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
        handled: bool,
        expected: Option<&Ty>,
    ) -> Ty {
        match &callee.kind {
            ExprKind::Name(ident) => {
                let name = self.name(ident);
                if let Some(ty) = self.local(name) {
                    self.outside_rl(span, || format!("call of local function `{name}`"));
                    return self.call_value(&ty, args, span, &format!("`{name}`"));
                }
                match self.env.lower(self.module, name) {
                    Some(Entity::Function(id)) => self.call_function(id, None, args, span, handled),
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
                self.call_value(&ty, args, span, "this function")
            }
        }
    }

    /// A call of a function value: positional arguments only (DECISION D-053).
    fn call_value(&mut self, ty: &Ty, args: &[Arg], span: Span, what: &str) -> Ty {
        let function = match self.resolve(ty) {
            Ty::Fn(function) => *function,
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
        for (arg, param) in args.iter().zip(&function.params) {
            self.expr_as(
                &arg.value,
                Some(param),
                Role::Argument {
                    param: "",
                    callee: what,
                },
            );
        }
        function.result
    }

    /// A call of a declared function, predicate or policy, with `receiver` as its first
    /// argument in method-call syntax.
    pub(crate) fn call_function(
        &mut self,
        id: usize,
        receiver: Option<(Span, Ty)>,
        args: &[Arg],
        span: Span,
        handled: bool,
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
        // DECISION D-056: declared effects and failures in an opaque predicate.
        if def.uses {
            self.impure(span, || {
                format!("call of {callee}, which uses capabilities,")
            });
        }
        if def.fails.is_some() && !handled {
            self.impure(span, || format!("call of {callee}, which can fail,"));
        }
        self.edge(id, span);
        if def.kind == FnKind::Policy {
            return self.call_policy(&def, &callee, receiver, args, span);
        }
        let subst = self.instantiate(&def.type_params, def.name, span);
        if !subst.is_empty() {
            self.instances.push((id, subst.clone(), span));
        }
        self.arguments(
            &callee,
            &def.params,
            &subst,
            receiver,
            args,
            span,
            def.module,
        );
        if def.requires {
            self.obligation(ObligationKind::Precondition, span, None);
        }
        if let Some(fails) = &def.fails {
            self.call_fails.insert(span, fails.clone());
        }
        def.result.base.substitute(&subst)
    }

    /// `p(v)`, a declassification (`spec/10 §7`): one argument, without a label, that flows
    /// into the policy's `from` type (DECISION D-053, DECISION D-066).
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
        let origin = def
            .params
            .first()
            .and_then(|p| self.origin(def.module, p.span));
        let value = match (receiver, args) {
            (Some((receiver_span, receiver_ty)), []) => {
                self.unify_at(
                    receiver_span,
                    &expected,
                    &receiver_ty,
                    Role::Argument { param: "", callee },
                );
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
                self.expr_as(
                    &arg.value,
                    Some(&expected),
                    Role::Argument { param: "", callee },
                );
                Some(arg.value.span)
            }
            _ => {
                self.report
                    .error(Code::U2001, span, format!("{callee} takes one argument"));
                self.unchecked(args);
                None
            }
        };
        if let (Some(value), Some(from)) = (value, from)
            && from.is_refined()
        {
            self.obligation(ObligationKind::Refinement, value, origin);
        }
        if let Some(fails) = &def.fails {
            self.call_fails.insert(span, fails.clone());
        }
        def.result.base.clone()
    }

    /// `@c.op(args)`, typed with the operation of the capability bound to `@c`; an unbound
    /// `@c` is U3001 (M5) (DECISION D-047).
    pub(crate) fn cap_op(
        &mut self,
        cap: &Ident,
        op: &Ident,
        args: &[Arg],
        span: Span,
        _handled: bool,
    ) -> Ty {
        let cap_name = self.name(cap);
        let op_name = self.name(op);
        self.outside_rl(span, || format!("operation `@{cap_name}.{op_name}`"));
        self.impure(span, || format!("operation `@{cap_name}.{op_name}`"));
        let capability = self
            .cap(cap_name)
            .and_then(|id| self.env.capabilities.get(id))
            .cloned();
        let Some(capability) = capability else {
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
        let callee = format!("operation `{}.{op_name}`", capability.name);
        self.arguments(
            &callee,
            &def.params,
            &[],
            None,
            args,
            span,
            capability.module,
        );
        if def.requires {
            self.obligation(ObligationKind::Precondition, span, None);
        }
        if let Some(fails) = &def.fails {
            self.call_fails.insert(span, fails.clone());
        }
        def.result.base
    }

    /// The parameter an unlabeled argument names by punning (`spec/04 §3` rule 3): a bare name
    /// equal to a parameter's name counts as labeled with it (DECISION D-064).
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
    /// a repeated parameter U2006, and a wrong number of arguments U2001 (DECISION D-067).
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
    ) {
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
            return;
        }
        if let (Some((receiver_span, receiver_ty)), Some(first)) = (&receiver, params.first()) {
            let expected = first.ty.base.substitute(subst);
            let role = Role::Argument {
                param: first.name,
                callee,
            };
            self.unify_at(*receiver_span, &expected, receiver_ty, role);
            if first.ty.is_refined() {
                let origin = self.origin(module, first.span);
                self.obligation(ObligationKind::Refinement, *receiver_span, origin);
            }
        }
        let mut taken: Vec<bool> = params.iter().map(|_| false).collect();
        if let Some(first) = taken.first_mut() {
            *first = receiver.is_some();
        }
        let mut targets: Vec<Option<usize>> = Vec::new();
        let mut problem = false;
        for (index, arg) in args.iter().enumerate() {
            let position = index + offset;
            let target = if let Some(label) = &arg.label {
                let label_name = self.name(label);
                match names.iter().position(|name| *name == label_name) {
                    Some(target) => Some(target),
                    None => {
                        self.unknown_label(callee, &names, label);
                        None
                    }
                }
            } else if let Some(target) = self.punned(arg, &names) {
                Some(target)
            } else if position == 0 {
                Some(0)
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
            self.expr_as(&arg.value, Some(&expected), role);
            if param.ty.is_refined() {
                let origin = self.origin(module, param.span);
                self.obligation(ObligationKind::Refinement, arg.value.span, origin);
            }
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
    }

    /// U2005: a label that names no parameter.
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
            None => {
                let listed: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
                diagnostic.repairs.push(Report::advice(format!(
                    "the parameters are {}",
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
                self.construct(kind, Some(index), &targs, args, ident.span, span);
                return Ty::Kind(kind, targs);
            }
            Some(Entity::Kind(id)) if self.env.kinds.get(id).is_some_and(|k| !k.is_sum()) => {
                let (params, kind_name) = self.kind_params(id);
                let targs = self.instantiate(&params, kind_name, ident.span);
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

    /// A constructor application, also after `fail` (`spec/04 §3` rule 4): with two or more
    /// fields every argument is labeled or punned, in any order, and one field may go without
    /// a label. Missing and unknown fields are U2009, a repeated field U2006, and an unlabeled
    /// argument where labels are required U2004 (DECISION D-064). `span` covers the name and
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
                    if let Some(close) = closest(label_name, names.iter().copied()) {
                        diagnostic.repairs.push(self.report.edit(
                            format!("write `{close}`"),
                            label.span,
                            close,
                        ));
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
            self.expr_as(&arg.value, Some(&expected), role);
            if field.ty.is_refined() {
                let origin = self.origin(module, field.span);
                self.obligation(ObligationKind::Refinement, arg.value.span, origin);
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
    }
}

/// The arguments with their targets, lambdas after the other arguments, each group in source
/// order (DECISION D-069).
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
