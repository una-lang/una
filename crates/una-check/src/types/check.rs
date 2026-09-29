//! The checker of one declaration at a time: what a position allows, locals, capability
//! bindings, and the rules for each form of declaration.

use std::collections::BTreeMap;

use una_diag::{Code, ObligationKind, Repair};
use una_syntax::Span;
use una_syntax::ast::{
    CapabilityDecl, ClauseKind, DeclKind, ExampleDecl, ExpectationKind, Expr, FunctionDecl,
    HandlerCall, HandlerDecl, Ident, OpClauseKind, PolicyDecl, PredicateDecl,
};

use super::authority::{Base, Bound, Collected, Frame, Pass};
use super::env::{Entity, Env, FnDef, FnKind, HandlerDef};
use super::facts::KindFacts;
use super::ty::{CapId, Declared, KindId, Names, Show, Ty};
use super::unify::{Origin, Vars};
use super::{Findings, Report};

/// Whether expressions must be in the refinement language (`spec/03 §3`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rl {
    /// No restriction.
    Off,
    /// Finding out, without reporting, whether a `let` right-hand side is in the refinement
    /// language, for selfification (`spec/03 §7`).
    Probe,
    /// A `where`, `requires` or `ensures` clause, or the predicate of `checked` (U2014).
    Clause,
    /// The body of a `reflect` predicate (U2015).
    Reflect,
}

/// What the position of an expression allows.
#[derive(Clone, Debug)]
pub(crate) struct Ctx {
    /// The type of `result`, where `result` is allowed (U2012).
    pub(crate) result: Option<Ty>,
    /// The type of `it`, where `it` is allowed (U2013).
    pub(crate) it: Option<Ty>,
    /// The type of `error`, where `error` is allowed (U2013).
    pub(crate) error: Option<Ty>,
    /// The error kind in which `fail` finds its variant (DECISION D-048).
    pub(crate) fails: Option<Ty>,
    pub(crate) rl: Rl,
    /// Inside the body of an opaque predicate, which is pure and cannot fail (U2015).
    pub(crate) opaque: bool,
    /// The failure of the call about to be checked is handled, by `attempt` or `if error`.
    pub(crate) handled: bool,
    /// The expression about to be checked is the whole expression of a statement, `let` or
    /// final expression (`spec/04 §4`).
    pub(crate) top: bool,
    /// The expression about to be checked is directly an argument of a call (`spec/10 §9`).
    pub(crate) argument: bool,
    /// Inside the `do` line of an example, where effects and failures need no handling and
    /// may nest (`spec/08 §2` rule 5).
    pub(crate) in_do: bool,
    /// Inside a handler operation body or an example, where `std.testing` may be used
    /// (`spec/14 §4`).
    pub(crate) testing: bool,
}

impl Ctx {
    fn plain() -> Ctx {
        Ctx {
            result: None,
            it: None,
            error: None,
            fails: None,
            rl: Rl::Off,
            opaque: false,
            handled: false,
            top: false,
            argument: false,
            in_do: false,
            testing: false,
        }
    }
}

/// Where an expression stands, as far as calls care: the whole expression of its statement,
/// with its failure handled, or directly a call argument. Each flag applies to one expression.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Site {
    pub(crate) top: bool,
    pub(crate) handled: bool,
    pub(crate) argument: bool,
}

/// How a mismatch is described.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Role<'r> {
    Plain,
    /// A condition, a clause or an `expect` line, which must be `Bool`.
    Condition(&'r str),
    /// An argument of a call.
    Argument {
        param: &'r str,
        callee: &'r str,
    },
    /// A field of a constructor.
    Field {
        field: &'r str,
        kind: &'r str,
    },
    /// A returned value.
    Return(&'r str),
    /// The value of an annotated `let`.
    Annotation(&'r str),
}

/// A local name and its base type.
#[derive(Clone, Debug)]
pub(crate) struct Local<'a> {
    pub(crate) name: &'a str,
    pub(crate) ty: Ty,
}

/// What a normal return path must satisfy, for pending obligations.
#[derive(Clone, Debug)]
pub(crate) struct Returning {
    pub(crate) result: Declared,
    /// The written result type, when it is in the package.
    pub(crate) origin: Option<Span>,
    pub(crate) ensures: bool,
}

pub(crate) struct Checker<'c, 'a, 's> {
    pub(crate) env: &'c mut Env<'a>,
    pub(crate) facts: &'c KindFacts,
    pub(crate) report: &'c mut Report<'s>,
    pub(crate) findings: &'c mut Findings,
    pub(crate) module: usize,
    pub(crate) vars: Vars,
    pub(crate) type_params: Vec<&'a str>,
    pub(crate) locals: Vec<Local<'a>>,
    /// Capability frames: the body's own context first, then the `with` bindings of the
    /// statements around the expression checked (DECISION D-047).
    pub(crate) frames: Vec<Frame>,
    /// What the body's own context is.
    pub(crate) base: Base,
    /// Which pass runs (DECISION D-082).
    pub(crate) pass: Pass,
    /// The facts that pass 1 collects for inference.
    pub(crate) collected: &'c mut Collected,
    /// How many lambdas enclose the expression checked.
    pub(crate) lambda_depth: usize,
    /// The functions that calls resolve to, by the span of the call.
    pub(crate) call_targets: BTreeMap<Span, usize>,
    pub(crate) ctx: Ctx,
    pub(crate) returning: Option<Returning>,
    /// The error kinds of the calls checked so far, by the span of the call.
    pub(crate) call_fails: BTreeMap<Span, Ty>,
    /// The function or predicate whose body is checked.
    pub(crate) current: Option<usize>,
    /// Instantiations of generic functions: callee, type arguments and where.
    pub(crate) instances: Vec<(usize, Vec<Ty>, Span)>,
    /// The operand types of `==` and `!=`.
    pub(crate) compared: Vec<Ty>,
    /// Set when an expression checked under [`Rl::Probe`] is outside the refinement language.
    pub(crate) probe_failed: bool,
    /// The number of diagnostics when the declaration started.
    started: usize,
}

impl Names for Checker<'_, '_, '_> {
    fn kind_name(&self, id: KindId) -> String {
        self.env.kind_name(id).to_owned()
    }

    fn param_name(&self, index: usize) -> String {
        self.type_params
            .get(index)
            .map_or_else(|| "?".to_owned(), |name| (*name).to_owned())
    }
}

/// Checks every declaration of module `module` in `pass`.
pub(crate) fn module<'a>(
    env: &mut Env<'a>,
    facts: &KindFacts,
    report: &mut Report<'_>,
    findings: &mut Findings,
    collected: &mut Collected,
    pass: Pass,
    module: usize,
) {
    let Some((tree, entities)) = env.modules.get(module).map(|m| (m.tree, m.decls.clone())) else {
        return;
    };
    for (decl, entity) in tree.decls.iter().zip(entities) {
        let mut checker = Checker::new(env, facts, report, findings, collected, pass, module);
        checker.decl(&decl.kind, entity);
        checker.finish();
    }
}

impl<'c, 'a, 's> Checker<'c, 'a, 's> {
    fn new(
        env: &'c mut Env<'a>,
        facts: &'c KindFacts,
        report: &'c mut Report<'s>,
        findings: &'c mut Findings,
        collected: &'c mut Collected,
        pass: Pass,
        module: usize,
    ) -> Checker<'c, 'a, 's> {
        let started = report.count();
        Checker {
            env,
            facts,
            report,
            findings,
            module,
            vars: Vars::default(),
            type_params: Vec::new(),
            locals: Vec::new(),
            frames: Vec::new(),
            base: Base::Quiet,
            pass,
            collected,
            lambda_depth: 0,
            call_targets: BTreeMap::new(),
            ctx: Ctx::plain(),
            returning: None,
            call_fails: BTreeMap::new(),
            current: None,
            instances: Vec::new(),
            compared: Vec::new(),
            probe_failed: false,
            started,
        }
    }

    // ----- helpers -----

    /// The text of a name of this module.
    pub(crate) fn name(&self, ident: &Ident) -> &'a str {
        self.env.text(self.module, ident.name)
    }

    /// A type as written in Una, with solved variables replaced.
    pub(crate) fn show(&self, ty: &Ty) -> String {
        let resolved = self.vars.resolve(ty);
        Show(&resolved, self).to_string()
    }

    /// `ty` with its solved variables replaced.
    pub(crate) fn resolve(&self, ty: &Ty) -> Ty {
        self.vars.resolve(ty)
    }

    /// Unifies `found` with `expected`, reporting U2001 at `span` when they differ.
    pub(crate) fn unify_at(
        &mut self,
        span: Span,
        expected: &Ty,
        found: &Ty,
        role: Role<'_>,
    ) -> bool {
        if self.vars.unify(expected, found) {
            return true;
        }
        let (expected, found) = (self.show(expected), self.show(found));
        let message = match role {
            Role::Plain => format!("expected `{expected}`, found `{found}`"),
            Role::Condition(what) => format!("{what} must be `{expected}`, found `{found}`"),
            Role::Argument { param: "", callee } => {
                format!("argument of {callee} must be `{expected}`, found `{found}`")
            }
            Role::Argument { param, callee } => {
                format!("argument `{param}` of {callee} must be `{expected}`, found `{found}`")
            }
            Role::Field { field, kind } => {
                format!("field `{field}` of `{kind}` must be `{expected}`, found `{found}`")
            }
            Role::Return(function) => format!("`{function}` returns `{expected}`, found `{found}`"),
            Role::Annotation(name) => {
                format!("`{name}` is declared as `{expected}`, found `{found}`")
            }
        };
        self.report.error(Code::U2001, span, message);
        false
    }

    /// Binds a local name.
    pub(crate) fn bind(&mut self, name: &'a str, ty: Ty) {
        self.locals.push(Local { name, ty });
    }

    /// The type of the innermost local named `name`.
    pub(crate) fn local(&self, name: &str) -> Option<Ty> {
        self.locals
            .iter()
            .rev()
            .find(|local| local.name == name)
            .map(|local| local.ty.clone())
    }

    /// Runs `check` with the position context `ctx`, then restores the current one.
    pub(crate) fn within<R>(&mut self, ctx: Ctx, check: impl FnOnce(&mut Self) -> R) -> R {
        let saved = std::mem::replace(&mut self.ctx, ctx);
        let result = check(self);
        self.ctx = saved;
        result
    }

    /// A clause, condition or `expect` line: an expression of type `Bool`.
    pub(crate) fn condition(&mut self, expr: &Expr, what: &str) {
        let ty = self.expr(expr, None);
        self.unify_at(expr.span, &Ty::Bool, &ty, Role::Condition(what));
    }

    /// A form outside the refinement language at `span`: U2014 in a clause, U2015 in a
    /// `reflect` predicate (`spec/03 §3, §4`). The sub-expressions of the form are not
    /// reported again.
    pub(crate) fn outside_rl(&mut self, span: Span, construct: impl FnOnce() -> String) {
        self.outside_rl_with(span, construct, None);
    }

    /// [`Checker::outside_rl`] with a hint that ends the message, such as "; contracts call
    /// only predicates".
    pub(crate) fn outside_rl_hint(
        &mut self,
        span: Span,
        construct: impl FnOnce() -> String,
        hint: &str,
    ) {
        self.outside_rl_with(span, || format!("{}{hint}", construct()), None);
    }

    /// [`Checker::outside_rl`] with a repair.
    pub(crate) fn outside_rl_with(
        &mut self,
        span: Span,
        construct: impl FnOnce() -> String,
        repair: Option<Repair>,
    ) {
        let (code, message) = match self.ctx.rl {
            Rl::Off => return,
            Rl::Probe => {
                self.probe_failed = true;
                self.ctx.rl = Rl::Off;
                return;
            }
            Rl::Clause => {
                let (construct, hint) = split_hint(construct());
                (
                    Code::U2014,
                    format!("{construct} is outside the refinement language{hint}"),
                )
            }
            Rl::Reflect => {
                let (construct, hint) = split_hint(construct());
                (
                    Code::U2015,
                    format!(
                        "{construct} is outside the refinement language, to which `reflect` predicates are limited{hint}"
                    ),
                )
            }
        };
        let mut diagnostic = self.report.make(code, span, message);
        diagnostic.repairs.extend(repair);
        self.report.push(diagnostic);
        self.ctx.rl = Rl::Off;
    }

    /// Something impure or failing at `span`, which the body of an opaque predicate cannot
    /// contain (U2015, DECISION D-056).
    pub(crate) fn impure(&mut self, span: Span, what: impl FnOnce() -> String) {
        if self.ctx.opaque {
            let message = format!(
                "{} in the body of a predicate, which must be pure and cannot fail",
                what()
            );
            self.report.error(Code::U2015, span, message);
        }
    }

    /// U2001 for a value whose type inference has not determined where its type is needed,
    /// for a field, a method or a call (DECISION D-069).
    pub(crate) fn unknown_type(&mut self, span: Span) {
        self.report.error(
            Code::U2001,
            span,
            "the type of this value is not known here; annotate it, for example the lambda parameter or the `let` it comes from",
        );
    }

    /// Records a pending obligation (`spec/12 §3`).
    pub(crate) fn obligation(&mut self, kind: ObligationKind, span: Span, origin: Option<Span>) {
        self.findings.obligations.push((kind, span, origin));
    }

    /// The span of a declared type of module `module`, when it can be reported.
    pub(crate) fn origin(&self, module: usize, span: Span) -> Option<Span> {
        self.env.in_package(module).then_some(span)
    }

    /// Checks the arguments of a handler instance and returns the capability it implements.
    pub(crate) fn install(&mut self, call: &HandlerCall) -> Option<CapId> {
        let name = self.name(&call.name);
        match self.env.lower(self.module, name) {
            Some(Entity::Handler(id)) => {
                let (params, capability, module, uses) = match self.env.handlers.get(id) {
                    Some(handler) => (
                        handler.params.clone(),
                        handler.capability,
                        handler.module,
                        handler
                            .uses
                            .iter()
                            .map(|n| n.usage.clone())
                            .collect::<Vec<_>>(),
                    ),
                    None => (Vec::new(), None, self.module, Vec::new()),
                };
                let callee = format!("handler `{name}`");
                self.arguments(&callee, &params, &[], None, &call.args, call.span, module);
                // DECISION D-074: the handler's own `uses` are held where it is installed.
                let level = self.frames.len();
                self.needs_site(&callee, &uses, call.span, level);
                capability
            }
            other => {
                // An unknown name is U1001 (M3); another name is U3010 (DECISION D-074).
                if other.is_some() && self.pass == Pass::Check {
                    self.report.error(
                        Code::U3010,
                        call.name.span,
                        format!("`{name}` is not a handler; `with` and `given` install handlers"),
                    );
                }
                for arg in &call.args {
                    self.expr(&arg.value, None);
                }
                None
            }
        }
    }

    /// Records a normal return path at `span` (`spec/12 §3`).
    pub(crate) fn returned(&mut self, span: Span) {
        let Some(returning) = &self.returning else {
            return;
        };
        let (refined, origin, ensures) = (
            returning.result.is_refined(),
            returning.origin,
            returning.ensures,
        );
        if refined {
            self.obligation(ObligationKind::Refinement, span, origin);
        }
        if ensures {
            self.obligation(ObligationKind::Postcondition, span, None);
        }
    }

    /// Ends the declaration: records generic instantiations and comparisons, and reports
    /// types that inference left unknown when nothing else was reported (DECISION D-054).
    fn finish(self) {
        let Checker {
            report,
            findings,
            vars,
            instances,
            compared,
            current,
            started,
            ..
        } = self;
        for (callee, args, span) in instances {
            let args: Vec<Ty> = args.iter().map(|arg| vars.resolve(arg)).collect();
            findings.instances.push((current, callee, args, span));
        }
        if let Some(current) = current {
            for ty in compared {
                let mut params = Vec::new();
                vars.resolve(&ty).params(&mut params);
                if !params.is_empty() {
                    findings.compared.entry(current).or_default().extend(params);
                }
            }
        }
        if report.count() != started {
            return;
        }
        let mut seen: Vec<Span> = Vec::new();
        for (origin, span) in vars.unsolved() {
            if seen.contains(&span) {
                continue;
            }
            seen.push(span);
            let message = match origin {
                Origin::Element => {
                    "the element type of this empty list cannot be inferred; give it a type, as in `let xs: List[Int] = []`".to_owned()
                }
                Origin::Instance { param, of } => {
                    format!("type parameter `{param}` of `{of}` cannot be inferred here; give the value a type, for example with an annotated `let`")
                }
            };
            report.error(Code::U2001, span, message);
        }
    }

    // ----- declarations -----

    fn decl(&mut self, kind: &'a DeclKind, entity: Option<Entity>) {
        match (kind, entity) {
            (DeclKind::Kind(_), Some(Entity::Alias(id))) => self.refinement(id),
            (DeclKind::Function(function), Some(Entity::Function(id))) => {
                self.function(function, id);
            }
            (DeclKind::Predicate(predicate), Some(Entity::Function(id))) => {
                self.predicate(predicate, id);
            }
            (DeclKind::Policy(policy), Some(Entity::Function(id))) => self.policy(policy, id),
            (DeclKind::Capability(capability), Some(Entity::Capability(id))) => {
                self.capability_decl(capability, id);
            }
            (DeclKind::Handler(handler), Some(Entity::Handler(id))) => {
                self.handler(handler, id);
            }
            (DeclKind::Example(example), _) => self.example(example),
            _ => {}
        }
    }

    /// The `where` predicate of an alias kind: `Bool`, in the refinement language, over `it`.
    fn refinement(&mut self, id: usize) {
        let Some(alias) = self.env.aliases.get(id) else {
            return;
        };
        let Some(refinement) = alias.refinement else {
            return;
        };
        self.type_params = alias.params.clone();
        let ctx = Ctx {
            it: Some(alias.base.base.clone()),
            rl: Rl::Clause,
            ..Ctx::plain()
        };
        self.within(ctx, |checker| {
            checker.condition(refinement, "a `where` clause")
        });
    }

    fn function(&mut self, function: &'a FunctionDecl, id: usize) {
        let Some(def) = self.env.functions.get(id).cloned() else {
            return;
        };
        self.current = Some(id);
        self.type_params = def.type_params.clone();
        for param in &def.params {
            self.bind(param.name, param.ty.base.clone());
        }
        self.enter_function(&def);
        // Clauses are in the refinement language, where U2014 reports effects and failures.
        let base = std::mem::replace(&mut self.base, Base::Quiet);
        for clause in &function.clauses {
            match &clause.kind {
                ClauseKind::Requires(expr) => {
                    let ctx = Ctx {
                        rl: Rl::Clause,
                        ..Ctx::plain()
                    };
                    self.within(ctx, |checker| {
                        checker.condition(expr, "a `requires` clause")
                    });
                }
                ClauseKind::Ensures(expr) => {
                    let ctx = Ctx {
                        rl: Rl::Clause,
                        result: Some(def.result.base.clone()),
                        ..Ctx::plain()
                    };
                    self.within(ctx, |checker| {
                        checker.condition(expr, "an `ensures` clause")
                    });
                }
                ClauseKind::Uses(_) | ClauseKind::Fails(_) | ClauseKind::DerivedFrom(_) => {}
            }
        }
        self.base = base;
        self.returning = Some(Returning {
            result: def.result.clone(),
            origin: self.origin(def.module, def.result_span),
            ensures: def.ensures,
        });
        let ctx = Ctx {
            fails: def.fails.clone(),
            ..Ctx::plain()
        };
        let expected = def.result.base.clone();
        self.within(ctx, |checker| {
            checker.body(
                &function.body,
                Some(&expected),
                Role::Return(def.name),
                true,
            );
        });
    }

    fn predicate(&mut self, predicate: &'a PredicateDecl, id: usize) {
        let Some(def) = self.env.functions.get(id).cloned() else {
            return;
        };
        self.current = Some(id);
        for param in &def.params {
            self.bind(param.name, param.ty.base.clone());
        }
        if !matches!(def.result.base, Ty::Bool | Ty::Error) {
            // DECISION D-065: a predicate returns `Bool`.
            let found = self.show(&def.result.base);
            self.report.error(
                Code::U2001,
                predicate.result.span,
                format!(
                    "a predicate returns `Bool`, and `{}` returns `{found}`",
                    def.name
                ),
            );
        }
        let reflect = matches!(def.kind, FnKind::Predicate { reflect: true });
        let ctx = Ctx {
            rl: if reflect { Rl::Reflect } else { Rl::Off },
            opaque: !reflect,
            ..Ctx::plain()
        };
        // A result that is not `Bool` is reported once, above; the body is checked against it.
        let expected = def.result.base.clone();
        self.within(ctx, |checker| {
            checker.body(
                &predicate.body,
                Some(&expected),
                Role::Return(def.name),
                false,
            );
        });
    }

    /// The `via` function of a policy takes its `from` type and returns its `to` type
    /// (`spec/10 §7`, DECISION D-066); labels are M6.
    fn policy(&mut self, policy: &'a PolicyDecl, id: usize) {
        let Some(def) = self.env.functions.get(id).cloned() else {
            return;
        };
        let via = self.name(&policy.via);
        let Some(entity) = self.env.lower(self.module, via) else {
            return;
        };
        let Entity::Function(target) = entity else {
            self.report.error(
                Code::U2001,
                policy.via.span,
                format!("`via` names a function, and `{via}` is not one"),
            );
            return;
        };
        let Some(target) = self.env.functions.get(target).cloned() else {
            return;
        };
        let from = def.params.first().map_or(Ty::Error, |p| p.ty.base.clone());
        let to = def.result.base.clone();
        let fits = target.kind == FnKind::Function
            && target.type_params.is_empty()
            && target.params.len() == 1
            && target
                .params
                .iter()
                .all(|p| self.vars.unify(&p.ty.base, &from))
            && self.vars.unify(&target.result.base, &to);
        if !fits {
            let params: Vec<String> = target
                .params
                .iter()
                .map(|p| self.show(&p.ty.base))
                .collect();
            let message = format!(
                "`via` function `{via}` must take a `{}` and return a `{}`; it is `fn({}) -> {}`",
                self.show(&from),
                self.show(&to),
                params.join(", "),
                self.show(&target.result.base)
            );
            self.report.error(Code::U2001, policy.via.span, message);
        }
    }

    fn capability_decl(&mut self, capability: &'a CapabilityDecl, id: CapId) {
        let Some(def) = self.env.capabilities.get(id).cloned() else {
            return;
        };
        for (op, op_def) in capability.ops.iter().zip(&def.ops) {
            let mark = self.locals.len();
            for param in &op_def.params {
                self.bind(param.name, param.ty.base.clone());
            }
            for clause in &op.clauses {
                match &clause.kind {
                    OpClauseKind::Requires(expr) => {
                        let ctx = Ctx {
                            rl: Rl::Clause,
                            ..Ctx::plain()
                        };
                        self.within(ctx, |checker| {
                            checker.condition(expr, "a `requires` clause")
                        });
                    }
                    OpClauseKind::Ensures(expr) => {
                        let ctx = Ctx {
                            rl: Rl::Clause,
                            result: Some(op_def.result.base.clone()),
                            ..Ctx::plain()
                        };
                        self.within(ctx, |checker| {
                            checker.condition(expr, "an `ensures` clause")
                        });
                    }
                    OpClauseKind::Fails(_) | OpClauseKind::Accepts(_) => {}
                }
            }
            self.locals.truncate(mark);
        }
    }

    /// Handler operations are checked like functions whose signature comes from the
    /// capability's operation (`spec/10 §8`); a missing or mismatched operation is M5.
    fn handler(&mut self, handler: &'a HandlerDecl, id: usize) {
        let Some(def) = self.env.handlers.get(id).cloned() else {
            return;
        };
        self.base = Base::Declared;
        self.frames = vec![Frame {
            bindings: def
                .uses
                .iter()
                .zip(&handler.uses)
                .map(|(need, uses)| {
                    (
                        self.name(&uses.cap).to_owned(),
                        Bound::Cap {
                            capability: need.usage.capability,
                            ops: need.usage.capability.map(|_| need.usage.ops.clone()),
                        },
                    )
                })
                .collect(),
        }];
        self.handler_rules(handler, &def);
        let capability = def
            .capability
            .and_then(|capability| self.env.capabilities.get(capability))
            .cloned();
        for op in &handler.ops {
            let mark = self.locals.len();
            for param in &def.params {
                self.bind(param.name, param.ty.base.clone());
            }
            let name = self.name(&op.name);
            let op_def = capability
                .as_ref()
                .and_then(|capability| capability.ops.iter().find(|o| o.name == name));
            for (index, param) in op.params.iter().enumerate() {
                let ty = op_def
                    .and_then(|o| o.params.get(index))
                    .map_or(Ty::Error, |p| p.ty.base.clone());
                self.bind(self.name(param), ty);
            }
            let module = capability.as_ref().map_or(0, |c| c.module);
            self.returning = op_def.map(|o| Returning {
                result: o.result.clone(),
                origin: self.origin(module, o.result_span),
                ensures: o.ensures,
            });
            let ctx = Ctx {
                fails: op_def.and_then(|o| o.fails.clone()),
                testing: true,
                ..Ctx::plain()
            };
            let expected = op_def.map(|o| o.result.base.clone());
            self.within(ctx, |checker| {
                checker.body(&op.body, expected.as_ref(), Role::Return(name), true);
            });
            self.returning = None;
            self.locals.truncate(mark);
        }
    }

    fn example(&mut self, example: &'a ExampleDecl) {
        self.base = Base::Example;
        self.ctx.testing = true;
        self.frames = vec![Frame::default()];
        for given in &example.givens {
            let capability = self.install(&given.handler);
            let name = self.name(&given.cap);
            if let Some(frame) = self.frames.first_mut() {
                frame.bindings.push((
                    name.to_owned(),
                    Bound::Handler {
                        capability,
                        span: given.handler.name.span,
                    },
                ));
            }
        }
        self.ctx.in_do = true;
        let ty = self.expr(&example.action, None);
        self.ctx.in_do = false;
        let action = example.action.span;
        let fails: Vec<Ty> = self
            .call_fails
            .iter()
            .filter(|(span, _)| {
                span.file == action.file && span.start >= action.start && span.end <= action.end
            })
            .map(|(_, kind)| kind.clone())
            .collect();
        for expectation in &example.expectations {
            match &expectation.kind {
                ExpectationKind::Holds(expr) => {
                    let ctx = Ctx {
                        result: Some(ty.clone()),
                        testing: true,
                        ..Ctx::plain()
                    };
                    self.within(ctx, |checker| checker.condition(expr, "an `expect` line"));
                }
                ExpectationKind::Fails(variant) => self.expected_failure(variant, &fails),
                ExpectationKind::Performed { cap, op, .. } => self.performed(cap, op),
            }
        }
    }

    /// The own context of a function's body (DECISION D-071, D-072): its `uses`, declared, or
    /// inferred in pass 2; being inferred in pass 1; or empty for a public function without
    /// `uses`, which is pure.
    fn enter_function(&mut self, def: &FnDef<'a>) {
        self.base = if def.infer_uses {
            if self.pass == Pass::Collect {
                Base::Inferring
            } else {
                Base::Declared
            }
        } else if def.uses.is_empty() && def.public {
            Base::Pure
        } else {
            Base::Declared
        };
        self.frames = vec![Frame {
            bindings: def
                .uses
                .iter()
                .map(|need| {
                    (
                        need.usage.name.clone(),
                        Bound::Cap {
                            capability: need.usage.capability,
                            ops: need.usage.capability.map(|_| need.usage.ops.clone()),
                        },
                    )
                })
                .collect(),
        }];
    }

    /// U3010 and U3011 for a handler declaration (`spec/05 §5`, DECISION D-073).
    fn handler_rules(&mut self, handler: &'a HandlerDecl, def: &HandlerDef<'a>) {
        if self.pass != Pass::Check {
            return;
        }
        let for_name = self.name(&handler.capability);
        let Some(capability) = def
            .capability
            .and_then(|c| self.env.capabilities.get(c))
            .cloned()
        else {
            // An unknown name is U1001 (M3).
            if !self.env.upper(self.module, for_name).is_empty() {
                self.report.error(
                    Code::U3010,
                    handler.capability.span,
                    format!("`{for_name}` is not a capability; `for` names the capability that a handler implements"),
                );
            }
            return;
        };
        let implemented: Vec<&str> = handler.ops.iter().map(|op| self.name(&op.name)).collect();
        let missing: Vec<String> = capability
            .ops
            .iter()
            .filter(|op| !implemented.contains(&op.name))
            .map(|op| format!("`{}`", op.name))
            .collect();
        if !missing.is_empty() {
            let what = if missing.len() == 1 {
                "operation"
            } else {
                "operations"
            };
            let handler_name = self.name(&handler.name);
            self.report.error(
                Code::U3011,
                handler.name.span,
                format!(
                    "handler `{handler_name}` does not implement the {what} {} of `{}`",
                    missing.join(", "),
                    capability.name
                ),
            );
        }
        for op in &handler.ops {
            let name = self.name(&op.name);
            match capability.ops.iter().find(|o| o.name == name) {
                None => {
                    let names: Vec<&str> = capability.ops.iter().map(|o| o.name).collect();
                    let mut diagnostic = self.report.make(
                        Code::U3010,
                        op.name.span,
                        format!("capability `{}` has no operation `{name}`", capability.name),
                    );
                    if let Some(close) = super::closest(name, names) {
                        diagnostic.repairs.push(self.report.edit(
                            format!("write `{close}`"),
                            op.name.span,
                            close,
                        ));
                    }
                    self.report.push(diagnostic);
                }
                Some(declared) if declared.params.len() != op.params.len() => {
                    self.report.error(
                        Code::U3010,
                        op.name.span,
                        format!(
                            "operation `{name}` of `{}` takes {}, and this implementation names {}",
                            capability.name,
                            super::expr::count(declared.params.len(), "parameter"),
                            op.params.len()
                        ),
                    );
                }
                Some(_) => {}
            }
        }
    }

    /// `expect performed @x.op`: `@x` is bound by a `given` line, and `op` is an operation of
    /// its capability (DECISION D-075).
    fn performed(&mut self, cap: &Ident, op: &Ident) {
        if self.pass != Pass::Check {
            return;
        }
        let name = self.name(cap);
        let op_name = self.name(op);
        match self.resolve_cap(name) {
            super::authority::Resolved::Handler { capability, .. } => {
                if let Some(capability) = capability.and_then(|c| self.env.capabilities.get(c))
                    && !capability.ops.iter().any(|o| o.name == op_name)
                {
                    self.report.error(
                        Code::U3002,
                        op.span,
                        format!(
                            "capability `{}` has no operation `{op_name}`",
                            capability.name
                        ),
                    );
                }
            }
            _ => self.report.error(
                Code::U3001,
                cap.span,
                format!("no `given` line binds `@{name}`"),
            ),
        }
    }

    /// `expect fails V`: `V` is a variant of an error kind of the `do` expression, or of a
    /// kind in scope when none has it (DECISION D-048).
    fn expected_failure(&mut self, variant: &Ident, fails: &[Ty]) {
        let name = self.name(variant);
        let in_kind = fails.iter().any(|kind| match self.resolve(kind) {
            Ty::Kind(id, _) => self.variant_index(id, name).is_some(),
            _ => false,
        });
        let in_scope = self
            .env
            .upper(self.module, name)
            .iter()
            .any(|entity| matches!(entity, Entity::Variant(..)));
        if in_kind || in_scope || fails.iter().any(|kind| self.resolve(kind) == Ty::Error) {
            return;
        }
        let kinds: Vec<String> = fails
            .iter()
            .map(|kind| format!("`{}`", self.show(kind)))
            .collect();
        let message = if kinds.is_empty() {
            format!("cannot find variant `{name}` in this module, its imports or the prelude")
        } else {
            format!("cannot find variant `{name}` in {}", kinds.join(" or "))
        };
        self.report.error(Code::U1001, variant.span, message);
    }

    /// The position of variant `name` in kind `id`.
    pub(crate) fn variant_index(&self, id: KindId, name: &str) -> Option<usize> {
        self.env
            .kinds
            .get(id)
            .and_then(|kind| kind.variants.iter().position(|v| v.name == name))
    }

    /// Fresh variables for the type parameters of a generic kind or function.
    pub(crate) fn instantiate(&mut self, params: &[&str], of: &str, span: Span) -> Vec<Ty> {
        params
            .iter()
            .map(|param| {
                let origin = Origin::Instance {
                    param: (*param).to_owned(),
                    of: of.to_owned(),
                };
                self.vars.fresh(origin, span)
            })
            .collect()
    }
}

/// Splits the hint that [`Checker::outside_rl_hint`] appends, which starts at `; `.
fn split_hint(text: String) -> (String, String) {
    match text.find("; ") {
        Some(at) => {
            let (construct, hint) = text.split_at(at);
            (construct.to_owned(), hint.to_owned())
        }
        None => (text, String::new()),
    }
}
