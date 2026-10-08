//! The checker of one declaration at a time: what a position allows, locals, capability
//! bindings, and the rules for each form of declaration.

use std::collections::BTreeMap;

use una_diag::{Code, ObligationKind, Repair};
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{
    CapabilityDecl, ClauseKind, Convention, DeclKind, ExampleDecl, ExpectationKind, Expr,
    FunctionDecl, HandlerCall, HandlerDecl, Ident, OpClauseKind, PolicyDecl, PredicateDecl,
};

use super::authority::{Base, Bound, Collected, Frame, Pass};
use super::env::{Constraint, Entity, Env, FnAst, FnDef, FnKind, HandlerDef};
use super::facts::KindFacts;
use super::label::Label;
use super::records::{Records, Res};
use super::ty::{CapId, Declared, KindId, Names, Refinement, Show, Ty};
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
    /// In an `ensures` clause of a function, where `old` is allowed (`spec/17 §3`).
    pub(crate) ensures: bool,
    /// The type of `it`, where `it` is allowed (U2013).
    pub(crate) it: Option<Ty>,
    /// The type of `error`, where `error` is allowed (U2013).
    pub(crate) error: Option<Ty>,
    /// The error kind in which `fail` finds its variant.
    pub(crate) fails: Option<Ty>,
    pub(crate) rl: Rl,
    /// Inside the body of an opaque predicate, which is pure and cannot fail (U2015).
    pub(crate) opaque: bool,
    /// Opaque predicates may be called here besides opaque bodies: the predicate of `checked`,
    /// the `where` clause of a kind and the clauses of a capability operation (ADR 0042,
    /// U2016).
    pub(crate) opaque_calls: bool,
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
    /// The program counter label (`spec/07 §3`), and the condition that raised it last.
    pub(crate) pc: Label,
    pub(crate) pc_from: Option<Span>,
    /// Whether labels are checked: not in `via` bodies or `expect` lines.
    pub(crate) labels: bool,
    /// In a handler operation body, the operation's `accepts` label: the effect floor that
    /// raises the program counter of every effect in the body (ADR 0038 D4, `spec/07 §7`
    /// rule 4).
    pub(crate) floor: Option<Label>,
}

impl Ctx {
    pub(crate) fn plain() -> Ctx {
        Ctx {
            result: None,
            ensures: false,
            it: None,
            error: None,
            fails: None,
            rl: Rl::Off,
            opaque: false,
            opaque_calls: false,
            handled: false,
            top: false,
            argument: false,
            in_do: false,
            testing: false,
            pc: Label::public(),
            pc_from: None,
            labels: true,
            floor: None,
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
    /// Declared `let edit`, or an `edit` parameter (`spec/17 §2`).
    pub(crate) editable: bool,
    /// Where it was passed by `take`, until an assignment gives it a new value (`spec/17 §3`).
    pub(crate) taken: Option<Span>,
    /// How many lambdas enclosed its binding: inside a deeper lambda it is captured by value
    /// and not editable (`spec/17 §5`).
    pub(crate) lambda_depth: usize,
    /// Its declared type is refined, so each rebind carries an obligation (`spec/17 §5`).
    pub(crate) refined: bool,
}

/// What a normal return path must satisfy, for pending obligations.
#[derive(Clone, Debug)]
pub(crate) struct Returning {
    pub(crate) result: Declared,
    /// The written result type, when it is in the package.
    pub(crate) origin: Option<Span>,
    pub(crate) ensures: bool,
    /// For a policy's `via` function, whether the policy's `to` type is refined: its result is
    /// a `refinement` obligation too (ADR 0038 D5, `spec/12 §3`).
    pub(crate) via_refined: bool,
}

pub(crate) struct Checker<'c, 'a, 's> {
    pub(crate) env: &'c mut Env<'a>,
    pub(crate) facts: &'c KindFacts,
    pub(crate) report: &'c mut Report<'s>,
    pub(crate) findings: &'c mut Findings,
    pub(crate) module: usize,
    pub(crate) vars: Vars,
    pub(crate) type_params: Vec<&'a str>,
    pub(crate) locals: super::locals::Locals<'a>,
    /// Capability frames: the body's own context first, then the `with` bindings of the
    /// statements around the expression checked.
    pub(crate) frames: Vec<Frame>,
    /// What the body's own context is.
    pub(crate) base: Base,
    /// Which pass runs.
    pub(crate) pass: Pass,
    /// The facts that pass 1 collects for inference.
    pub(crate) collected: &'c mut Collected,
    /// How many lambdas enclose the expression checked.
    pub(crate) lambda_depth: usize,
    /// The argument about to be checked is one of a prelude function's.
    pub(crate) prelude_argument: bool,
    /// The innermost lambda is directly an argument of a prelude function, for the advice of
    /// U3004 in edition `0.1`.
    pub(crate) prelude_lambda: bool,
    /// Where the locals of the innermost enclosing `together` task start: a local before it
    /// is captured by the task, which cannot change it (U2023, `spec/19 §2` rule 3).
    pub(crate) task_mark: Option<usize>,
    /// The receiver of the method call being checked, with its call-site marker, for its
    /// parameter's convention and exclusivity (`spec/17 §3`).
    pub(crate) receiver_marker: Option<(Option<Convention>, Expr)>,
    /// The spans of the calls that are the whole expression of a statement, a `let` or an
    /// assignment: the only places for a call with `edit` or `take`, U3013 elsewhere
    /// (`spec/17 §3` rule 5), and where the desugar binds its tuple (`spec/17 §4`).
    pub(crate) statement_calls: Vec<Span>,
    /// How many of the enclosing lambdas are checked in their caller's context, as the
    /// arguments of `of` parameters (ADR 0062).
    pub(crate) inherited: usize,
    /// The callee whose arguments are being checked, when it has `of` clauses.
    pub(crate) of_callee: Option<super::effects::OfCallee>,
    /// The argument being checked for an `of` parameter, for the lambda or the name it is.
    pub(crate) of_argument: Option<super::effects::OfArgument>,
    /// The lambdas being checked in their caller's context, innermost last.
    pub(crate) of_lambdas: Vec<super::effects::OfLambda>,
    /// What the arguments of the calls with `of` clauses being checked contribute, innermost
    /// last.
    pub(crate) of_found: Vec<super::effects::OfFound>,
    /// The functions that calls resolve to, by the span of the call.
    pub(crate) call_targets: BTreeMap<Span, usize>,
    /// The types of the operands of the binary operations being checked.
    pub(crate) operands: Vec<Ty>,
    /// The spans of the variants written without their fields (U2009), for the `is` repair of
    /// a comparison (#205).
    pub(crate) needs_fields: Vec<Span>,
    /// In pass 1, the operations on references whose capability inference decides, by the
    /// span of the operation: the reference and the operation.
    pub(crate) pending_ops: BTreeMap<Span, (String, String)>,
    /// In pass 2, what this declaration resolves.
    pub(crate) records: Records,
    /// The span of the declaration's name, where its failure is recorded.
    pub(crate) decl_span: Option<Span>,
    pub(crate) ctx: Ctx,
    pub(crate) returning: Option<Returning>,
    /// The error kinds of the calls checked so far, by the span of the call.
    pub(crate) call_fails: BTreeMap<Span, Ty>,
    /// The labels that decide the failures of calls of prelude functions whose failure depends
    /// on their arguments' values, as `parse_json` (`spec/23 §5`, `spec/07 §3`), by the span of
    /// the call: handling such a failure branches on them (N-27).
    pub(crate) decided_failures: BTreeMap<Span, Label>,
    /// For each lambda, by its span, which of its parameters are annotated: the body assumes
    /// the annotation, whatever the lambda's type unifies with (#350).
    pub(crate) annotated_lambdas: BTreeMap<Span, Vec<bool>>,
    /// In pass 1, the labels that flow into the label variables of this declaration's editable
    /// bindings, by symbol (ADR 0075).
    pub(crate) edit_bounds: BTreeMap<String, Label>,
    /// In pass 1, the open positions of this declaration's editable bindings: each variable's
    /// name and the unification variable whose type will be its shape (ADR 0075).
    pub(crate) edit_open: Vec<(String, usize)>,
    /// The outer form of the editable binding being opened, for the example of an annotation.
    pub(crate) edit_outer: String,
    /// The function or predicate whose body is checked.
    pub(crate) current: Option<usize>,
    /// The handler whose operation bodies are checked (ADR 0043).
    pub(crate) handler: Option<usize>,
    /// Instantiations of generic functions: callee, type arguments and where.
    pub(crate) instances: Vec<(usize, Vec<Ty>, Span)>,
    /// What the operators on values of type parameters need: the operand type, the
    /// constraint that allows the operator, and where (`spec/20 §3` rule 1).
    pub(crate) needs: Vec<(Ty, Constraint, Span)>,
    /// The interpolations among `needs`, whose U2026 names interpolation (ADR 0068).
    pub(crate) interpolations: Vec<Span>,
    /// The labeled fields that interpolations wrote, by the text literal's span: the field,
    /// its kind and its declared label, which U4003 names at a sink (ADR 0068 D8).
    pub(crate) written_fields: Vec<(Span, String, String, Label)>,
    /// Set when an expression checked under [`Rl::Probe`] is outside the refinement language.
    pub(crate) probe_failed: bool,
    /// The instantiations of type parameters, judged when the declaration ends (U3016).
    pub(crate) instantiations: Vec<super::generic::Instantiation>,
    /// Where U3006 reports a function value that escapes; U3016 is not reported there again.
    pub(crate) escaped: Vec<Span>,
    /// The statement whose expression is checked, for repairs that add a `with` binding.
    pub(crate) statement: Option<Statement>,
    /// In a `do` line, the operands of `attempt` and the lambdas, whose failures do not reach
    /// the `do` expression's error kind (D-170).
    pub(crate) absorbed: Vec<Span>,
    /// The number of diagnostics when the declaration started.
    started: usize,
}

/// A statement `line_expr [ with_clause ]` whose expression is checked (`spec/05 §3`).
#[derive(Clone, Debug)]
pub(crate) struct Statement {
    /// The span of its line expression.
    pub(crate) expr: Span,
    /// Where a new `with` binding is inserted: after the expression or the last binding.
    pub(crate) insert: Span,
    /// Whether it has a `with` clause already.
    pub(crate) with: bool,
    /// The references its `with` clause binds.
    pub(crate) bound: Vec<String>,
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
        checker.decl_span = Some(decl.span);
        checker.decl(&decl.kind, entity);
        checker.finish();
    }
}

/// Where each function of the package is declared: the module and the index in its
/// declarations, of each declaration that defines it (repeats included), in module order, for
/// modules whose diagnostics are reported. Built once, so that checking one function finds it
/// in O(log n) instead of scanning every declaration (#375).
pub(crate) fn places(env: &Env<'_>) -> std::collections::BTreeMap<usize, Vec<(usize, usize)>> {
    let mut places: std::collections::BTreeMap<usize, Vec<(usize, usize)>> =
        std::collections::BTreeMap::new();
    for module in 0..env.prelude {
        if !env.reports(module) {
            continue;
        }
        let Some(m) = env.modules.get(module) else {
            continue;
        };
        for (index, entity) in m.decls.iter().enumerate() {
            if let Some(Entity::Function(id)) = entity {
                places.entry(*id).or_default().push((module, index));
            }
        }
    }
    places
}

/// Checks the declarations at `at`, as [`places`] gives them, each as a module's check does,
/// without scanning the package (#375).
pub(crate) fn declarations_at<'a>(
    env: &mut Env<'a>,
    facts: &KindFacts,
    report: &mut Report<'_>,
    findings: &mut Findings,
    collected: &mut Collected,
    pass: Pass,
    at: &[(usize, usize)],
) {
    for &(module, index) in at {
        let Some((tree, entity)) = env
            .modules
            .get(module)
            .map(|m| (m.tree, m.decls.get(index).copied().flatten()))
        else {
            continue;
        };
        let Some(decl) = tree.decls.get(index) else {
            continue;
        };
        let mut checker = Checker::new(env, facts, report, findings, collected, pass, module);
        checker.decl_span = Some(decl.span);
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
        let kinds = env.kind_labels.clone();
        Checker {
            env,
            facts,
            report,
            findings,
            module,
            vars: Vars::new(kinds),
            type_params: Vec::new(),
            locals: super::locals::Locals::default(),
            frames: Vec::new(),
            base: Base::Quiet,
            pass,
            collected,
            receiver_marker: None,
            statement_calls: Vec::new(),
            inherited: 0,
            of_callee: None,
            of_argument: None,
            of_lambdas: Vec::new(),
            of_found: Vec::new(),
            lambda_depth: 0,
            prelude_argument: false,
            prelude_lambda: false,
            task_mark: None,
            call_targets: BTreeMap::new(),
            operands: Vec::new(),
            needs_fields: Vec::new(),
            pending_ops: BTreeMap::new(),
            records: Records::default(),
            decl_span: None,
            ctx: Ctx::plain(),
            returning: None,
            call_fails: BTreeMap::new(),
            decided_failures: BTreeMap::new(),
            annotated_lambdas: BTreeMap::new(),
            edit_bounds: BTreeMap::new(),
            edit_open: Vec::new(),
            edit_outer: String::new(),
            current: None,
            handler: None,
            instances: Vec::new(),
            needs: Vec::new(),
            interpolations: Vec::new(),
            written_fields: Vec::new(),
            probe_failed: false,
            instantiations: Vec::new(),
            escaped: Vec::new(),
            statement: None,
            absorbed: Vec::new(),
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

    /// A written type as Una names it: a refined type by its refined kind or range, without
    /// its label.
    pub(crate) fn show_declared(&self, declared: &Declared) -> String {
        match declared.refinements.last() {
            Some(Refinement::Alias(id, _)) => self
                .env
                .aliases
                .get(*id)
                .map_or_else(|| self.show(&declared.base), |alias| alias.name.to_owned()),
            Some(Refinement::Range(range, _)) => format!("{} {range}", self.show(&declared.base)),
            None => self.show(&declared.base),
        }
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
        self.bind_as(name, ty, false);
    }

    /// Binds a local name, editable when `editable` (`spec/17 §2`).
    pub(crate) fn bind_as(&mut self, name: &'a str, ty: Ty, editable: bool) {
        self.locals.push(Local {
            name,
            ty,
            editable,
            taken: None,
            lambda_depth: self.lambda_depth,
            refined: false,
        });
    }

    /// The type of the innermost local named `name`.
    pub(crate) fn local(&self, name: &str) -> Option<Ty> {
        self.locals.find(name).map(|local| local.ty.clone())
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
        self.condition_label(expr, what);
    }

    /// A condition of a branch, and its label, which raises the program counter of the
    /// branches (`spec/07 §3` rule 2).
    pub(crate) fn condition_label(&mut self, expr: &Expr, what: &str) -> Label {
        let ty = self.expr(expr, None);
        self.unify_at(expr.span, &Ty::Bool, &ty, Role::Condition(what));
        self.labeled(&ty).top_label()
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
    /// contain (U2015).
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
    /// for a field, a method or a call.
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
                self.resolved(call.span, Res::Handler { id });
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
                let checked =
                    self.arguments(&callee, &params, &[], None, &call.args, call.span, module);
                self.argument_flows(&callee, checked);
                // The handler's own `uses` are held where it is installed.
                let level = self.frames.len();
                self.needs_site(&callee, &uses, call.span, level);
                // Its effects run under the program counter of the statement.
                self.call_under_pc(call.span, &callee, &uses, false, true, &Label::public());
                capability
            }
            other => {
                // An unknown name is U1001 (M3); another name is U3010.
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

    /// Records, in pass 2, what the expression or pattern at `span` resolves to.
    pub(crate) fn resolved(&mut self, span: Span, res: Res) {
        if self.pass == Pass::Check {
            self.records.res.insert(span, res);
        }
    }

    /// Records, in pass 2, the parameter or field of each argument of the call at `span`.
    pub(crate) fn ordered(&mut self, span: Span, order: Vec<Option<usize>>) {
        if self.pass == Pass::Check {
            self.records.args.insert(span, order);
        }
    }

    /// Records, in pass 2, the written type of the annotated `let` whose name is at `span`.
    pub(crate) fn annotated(&mut self, span: Span, declared: &Declared) {
        if self.pass == Pass::Check {
            self.records.annotations.insert(span, declared.clone());
        }
    }

    /// Records, in pass 2, the refinements of the lambda parameter whose name is at `span`
    /// (ADR 0037 rule 1).
    pub(crate) fn lambda_refinements(&mut self, span: Span, refinements: Vec<Refinement>) {
        if self.pass == Pass::Check && !refinements.is_empty() {
            self.records.lambda_params.insert(span, refinements);
        }
    }

    /// Records, in pass 2, the type of the expression or binder at `span`.
    pub(crate) fn typed(&mut self, span: Span, ty: &Ty) {
        if self.pass == Pass::Check {
            self.records.types.insert(span, ty.clone());
        }
    }

    /// Records a normal return path at `span` (`spec/12 §3`).
    pub(crate) fn returned(&mut self, span: Span) {
        let Some(returning) = &self.returning else {
            return;
        };
        let (refined, origin, ensures) = (
            returning.result.is_refined() || returning.via_refined,
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

    /// Ends the declaration: records generic instantiations and what operators on type
    /// parameters need, and reports
    /// types that inference left unknown when nothing else was reported.
    fn finish(self) {
        let Checker {
            report,
            findings,
            vars,
            instances,
            needs,
            interpolations,
            current,
            env,
            started,
            records,
            decl_span,
            pass,
            instantiations,
            mut escaped,
            edit_bounds,
            edit_open,
            ..
        } = self;
        // ADR 0075 rule 2: pass 1 solves the label variables of the editable bindings.
        if pass == Pass::Collect
            && !edit_bounds.is_empty()
            && let Some(name) = super::edit_labels::solve_into(&edit_bounds, &mut env.edit_labels)
        {
            env.edit_labels_changed = true;
            env.edit_labels_changed_name = Some(name);
        }
        if pass == Pass::Collect
            && Checker::record_edit_shapes(&edit_open, &vars, &mut env.edit_shapes)
        {
            env.edit_labels_changed = true;
        }
        if pass == Pass::Check {
            super::generic::second_class(report, &vars, &instantiations, &mut escaped);
            // Each solved variable resolved once, however many types name it (ADR 0077 3a).
            let resolver = vars.resolver();
            findings
                .records
                .absorb(records, |ty| resolver.resolve_labeled(ty));
            if report.count() != started
                && let Some(span) = decl_span
            {
                findings.failed.insert(span);
            }
        }
        for (callee, args, span) in instances {
            let args: Vec<Ty> = args.iter().map(|arg| vars.resolve(arg)).collect();
            findings.instances.push((current, callee, args, span));
        }
        if let Some(current) = current {
            let v0_2 = env.edition >= Edition::V0_2;
            let mut reported: Vec<Span> = Vec::new();
            // The U2026 of each type parameter, in order: one edit per parameter, writing the
            // strongest constraint its body needs, so that the repairs of one check never edit
            // the same text (#257).
            let mut unmet: Vec<(usize, Constraint, Span)> = Vec::new();
            for (ty, need, span) in needs {
                let resolved = vars.resolve(&ty);
                let mut params = Vec::new();
                match need {
                    // Equality is structural: it compares every value of a type parameter
                    // that the operands hold.
                    Constraint::Eq | Constraint::None => resolved.params(&mut params),
                    Constraint::Ord | Constraint::Integer => {
                        if let Ty::Param(index) = resolved.unlabeled() {
                            params.push(*index);
                        }
                    }
                }
                for index in params {
                    if !v0_2 {
                        // Edition 0.1: the inferred bound of `spec/10 §12` rule 9.
                        findings.compared.entry(current).or_default().insert(index);
                        continue;
                    }
                    let Some(def) = env.functions.get(current) else {
                        continue;
                    };
                    let written = def.constraints_written.get(index).copied().unwrap_or(false);
                    let have = def.constraints.get(index).copied().unwrap_or_default();
                    if !def.public && !written {
                        // Inferred for a private function (`spec/20 §3` rule 4).
                        let entry = findings
                            .needs
                            .entry(current)
                            .or_default()
                            .entry(index)
                            .or_default();
                        *entry = (*entry).max(need);
                    } else if have < need && pass == Pass::Check && !reported.contains(&span) {
                        reported.push(span);
                        unmet.push((index, need, span));
                    }
                }
            }
            if let Some(def) = env.functions.get(current) {
                let mut edited: Vec<usize> = Vec::new();
                for &(index, need, span) in &unmet {
                    let strongest = unmet
                        .iter()
                        .filter(|(other, _, _)| *other == index)
                        .map(|(_, need, _)| *need)
                        .max()
                        .unwrap_or(need);
                    let edit = !edited.contains(&index);
                    edited.push(index);
                    let interpolated = interpolations.contains(&span);
                    report.push(unconstrained(
                        report,
                        def,
                        index,
                        need,
                        span,
                        interpolated,
                        (strongest, edit),
                    ));
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
            labels: false,
            opaque_calls: true,
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
            let ty = self.symbolic(&param.ty.base);
            let editable = param.convention == Some(una_syntax::ast::Convention::Edit);
            self.bind_as(param.name, ty, editable);
            if let Some(local) = self.locals.last_mut() {
                local.refined = editable && param.ty.is_refined();
            }
        }
        self.enter_function(&def);
        if self.pass == Pass::Check {
            let names: Vec<(&str, Span)> = function
                .clauses
                .iter()
                .filter_map(|clause| match &clause.kind {
                    ClauseKind::Uses(uses) => Some((self.name(&uses.cap), uses.cap.span)),
                    _ => None,
                })
                .collect();
            super::authority::duplicate_refs(self.report, &names);
        }
        // Clauses are in the refinement language, where U2014 reports effects and failures.
        let base = std::mem::replace(&mut self.base, Base::Quiet);
        for clause in &function.clauses {
            match &clause.kind {
                ClauseKind::Requires(expr) => {
                    let ctx = Ctx {
                        rl: Rl::Clause,
                        labels: false,
                        ..Ctx::plain()
                    };
                    self.within(ctx, |checker| {
                        checker.condition(expr, "a `requires` clause")
                    });
                }
                ClauseKind::Ensures(expr) => {
                    let ctx = Ctx {
                        rl: Rl::Clause,
                        labels: false,
                        result: Some(def.result.base.clone()),
                        ensures: true,
                        ..Ctx::plain()
                    };
                    self.within(ctx, |checker| {
                        checker.condition(expr, "an `ensures` clause")
                    });
                }
                ClauseKind::Uses(_)
                | ClauseKind::Fails(_)
                | ClauseKind::UsesOf(_)
                | ClauseKind::FailsOf(_)
                | ClauseKind::DerivedFrom(_) => {}
            }
        }
        self.base = base;
        let via_refined = self
            .env
            .vias
            .get(&id)
            .and_then(|policy| self.env.functions.get(*policy))
            .is_some_and(|policy| policy.result.is_refined());
        self.returning = Some(Returning {
            result: def.result.clone(),
            origin: self.origin(def.module, def.result_span),
            ensures: def.ensures,
            via_refined,
        });
        // The body of a `via` function is not checked for labels.
        let ctx = Ctx {
            fails: def.fails.clone().or_else(|| {
                // `fails of p`: the body fails with the kind of the argument, a hidden
                // parameter here (ADR 0062).
                def.fails_of.map(|_| Ty::Param(def.type_params.len()))
            }),
            labels: !self.env.vias.contains_key(&id),
            ..Ctx::plain()
        };
        let expected = self.symbolic(&def.result.base);
        self.within(ctx, |checker| {
            checker.body(
                &function.body,
                Some(&expected),
                Role::Return(def.name),
                true,
            );
        });
        // The final value of an `edit` parameter returns to the caller, a use of it after the
        // body: one still taken there is U2017 (`spec/17 §3` rules 1 and 3).
        if self.pass == Pass::Check {
            for (param, local) in def.params.iter().zip(self.locals.clone()) {
                if param.convention != Some(una_syntax::ast::Convention::Edit) {
                    continue;
                }
                if let Some(taken) = local.taken {
                    self.report.error(
                        Code::U2017,
                        taken,
                        format!(
                            "`{}` is an `edit` parameter, whose final value returns to the caller, so it cannot stay taken; give it a new value before the end of the body",
                            param.name
                        ),
                    );
                }
            }
        }
    }

    fn predicate(&mut self, predicate: &'a PredicateDecl, id: usize) {
        let Some(def) = self.env.functions.get(id).cloned() else {
            return;
        };
        self.current = Some(id);
        for param in &def.params {
            self.bind(param.name, param.ty.base.clone());
        }
        if !matches!(def.result.base.unlabeled(), Ty::Bool | Ty::Error) {
            // A predicate returns `Bool`.
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
        let (body_span, name) = (predicate_body_span(&predicate.body), def.name);
        self.within(ctx, |checker| {
            let found = checker.body(
                &predicate.body,
                Some(&expected),
                Role::Return(def.name),
                false,
            );
            checker.flow(
                body_span,
                &found,
                &expected,
                &format!("the result of `{name}`"),
            );
        });
    }

    /// The `via` function of a policy takes its `from` type and returns its `to` type
    /// (`spec/10 §7`); labels are M6.
    fn policy(&mut self, policy: &'a PolicyDecl, id: usize) {
        let Some(def) = self.env.functions.get(id).cloned() else {
            return;
        };
        // A policy lives in a module whose header says `trusted`; a header
        // without the listing in `una.toml` is U7005.
        let trusted = self
            .env
            .modules
            .get(self.module)
            .and_then(|m| m.tree.module.as_ref())
            .is_some_and(|header| header.trusted);
        if !trusted && self.pass == Pass::Check {
            let path = self
                .env
                .modules
                .get(self.module)
                .map_or("", |m| m.path.as_str());
            self.report.error(
                Code::U4004,
                policy.name.span,
                format!(
                    "policy `{}` is declared in module `{path}`, which is not trusted; declare `trusted module {path}` and list it under `[trusted] modules` in `una.toml`",
                    def.name
                ),
            );
        }
        let via = self.name(&policy.via);
        let Some(entity) = self.env.lower(self.module, via) else {
            return;
        };
        let Entity::Function(target_id) = entity else {
            self.report.error(
                Code::U4005,
                policy.via.span,
                format!("`via` names a function, and `{via}` is not one"),
            );
            return;
        };
        let Some(target) = self.env.functions.get(target_id).cloned() else {
            return;
        };
        if (target.public || target.module != self.module) && self.pass == Pass::Check {
            self.report.error(
                Code::U4005,
                policy.via.span,
                format!(
                    "the `via` function of a policy is private to its trusted module, and `{via}` is not; remove its `pub`, or declare it in this module"
                ),
            );
        }
        let from = def
            .params
            .first()
            .map_or_else(|| Declared::plain(Ty::Error), |p| p.ty.clone());
        let to = def.result.clone();
        let fits = target.kind == FnKind::Function
            && target.type_params.is_empty()
            && target.params.len() == 1
            && target
                .params
                .iter()
                .all(|p| self.vars.unify(&p.ty.base, &from.base))
            && self.vars.unify(&target.result.base, &to.base);
        // ADR 0047 rule 1: the types match except for labels, refinements included, so the
        // argument of a policy call, checked against `from`, meets every assumption of the
        // `via` body, and its result is the `to` type.
        let refined = fits
            && target.params.iter().all(|p| same_refinements(&p.ty, &from))
            && same_refinements(&target.result, &to);
        if !fits || !refined {
            let expected = format!(
                "fn({}) -> {}",
                self.show_declared(&from),
                self.show_declared(&to)
            );
            // The `via` function's type parameters show by their names (final review F-004).
            let saved = std::mem::replace(&mut self.type_params, target.type_params.clone());
            let params: Vec<String> = target
                .params
                .iter()
                .map(|p| self.show_declared(&p.ty))
                .collect();
            let found = format!(
                "fn({}) -> {}",
                params.join(", "),
                self.show_declared(&target.result)
            );
            self.type_params = saved;
            let message = format!(
                "`via` function `{via}` must have type `{expected}`, except for labels; it is `{found}`"
            );
            // Types other than labels match between a policy and its `via`
            // function (`spec/07 §5` rule 4).
            self.report.error(Code::U4005, policy.via.span, message);
        }
        // ADR 0047 rule 2: a policy call checks its argument against `from` only, so a `via`
        // function has no precondition.
        if let FnAst::Function(decl) = target.ast
            && self.pass == Pass::Check
        {
            for clause in &decl.clauses {
                if matches!(clause.kind, ClauseKind::Requires(_)) {
                    self.report.error(
                        Code::U4005,
                        clause.span,
                        format!(
                            "`via` function `{via}` of policy `{}` has a `requires` clause, which a policy call never checks; remove it and handle every value of the `from` type",
                            def.name
                        ),
                    );
                }
            }
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
                            labels: false,
                            opaque_calls: true,
                            ..Ctx::plain()
                        };
                        self.within(ctx, |checker| {
                            checker.condition(expr, "a `requires` clause")
                        });
                    }
                    OpClauseKind::Ensures(expr) => {
                        let ctx = Ctx {
                            rl: Rl::Clause,
                            labels: false,
                            opaque_calls: true,
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
        self.handler = Some(id);
        self.base = Base::Declared;
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
                            clause: Some(need.span),
                        },
                    )
                })
                .collect(),
        }];
        self.handler_rules(handler, &def);
        if self.pass == Pass::Check {
            let names: Vec<(&str, Span)> = handler
                .uses
                .iter()
                .map(|uses| (self.name(&uses.cap), uses.cap.span))
                .collect();
            super::authority::duplicate_refs(self.report, &names);
        }
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
            // Callers may pass anything up to `accepts`.
            // D-141: an operation that accepts every label has the effect floor `⊤`
            // (ADR 0038 D4). `Keys.hmac` accepts `public`; its `data` is carried (ADR 0039).
            let accepts = op_def
                .and_then(|o| o.accepts.clone())
                .unwrap_or_else(|| self.top());
            // ADR 0039: a carried parameter (`data` of `Keys.hmac`) may be of every label,
            // and the result may carry it.
            let top = self.top();
            let carried = |index: usize| op_def.is_some_and(|o| o.carried.contains(&index));
            for (index, param) in op.params.iter().enumerate() {
                let raise = if carried(index) { &top } else { &accepts };
                let ty = op_def
                    .and_then(|o| o.params.get(index))
                    .map_or(Ty::Error, |p| p.ty.base.raise_deep(raise));
                self.bind(self.name(param), ty);
            }
            let module = capability.as_ref().map_or(0, |c| c.module);
            self.returning = op_def.map(|o| Returning {
                result: o.result.clone(),
                origin: self.origin(module, o.result_span),
                ensures: o.ensures,
                via_refined: false,
            });
            let ctx = Ctx {
                fails: op_def.and_then(|o| o.fails.clone()),
                testing: true,
                floor: Some(accepts.clone()),
                ..Ctx::plain()
            };
            let expected = op_def.map(|o| {
                if o.carried.is_empty() {
                    o.result.base.clone()
                } else {
                    o.result.base.clone().with_label(&top)
                }
            });
            self.within(ctx, |checker| {
                checker.body(&op.body, expected.as_ref(), Role::Return(name), true);
            });
            self.returning = None;
            self.locals.truncate(mark);
        }
        self.handler = None;
    }

    fn example(&mut self, example: &'a ExampleDecl) {
        self.base = Base::Example;
        self.ctx.testing = true;
        self.frames = vec![Frame::default()];
        if self.pass == Pass::Check {
            let names: Vec<(&str, Span)> = example
                .givens
                .iter()
                .map(|given| (self.name(&given.cap), given.cap.span))
                .collect();
            super::authority::duplicate_refs(self.report, &names);
        }
        for given in &example.givens {
            let capability = self.install(&given.handler);
            let name = self.name(&given.cap);
            // The first `given` of a name wins.
            if let Some(frame) = self.frames.first_mut()
                && !frame.bindings.iter().any(|(bound, _)| bound == name)
            {
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
        self.absorbed.clear();
        let ty = self.expr(&example.action, None);
        self.ctx.in_do = false;
        // The error kind of the `do` expression: the failures of its calls, except those that
        // `attempt` absorbs or that belong to a lambda (D-170).
        let action = example.action.span;
        let within = |span: &Span, outer: &Span| {
            span.file == outer.file && span.start >= outer.start && span.end <= outer.end
        };
        let fails: Vec<Ty> = self
            .call_fails
            .iter()
            .filter(|(span, _)| {
                within(span, &action) && !self.absorbed.iter().any(|outer| within(span, outer))
            })
            .map(|(_, kind)| kind.clone())
            .collect();
        for expectation in &example.expectations {
            match &expectation.kind {
                ExpectationKind::Holds(expr) => {
                    // `expect` lines are not checked for labels.
                    let ctx = Ctx {
                        result: Some(ty.clone()),
                        testing: true,
                        labels: false,
                        ..Ctx::plain()
                    };
                    self.within(ctx, |checker| checker.condition(expr, "an `expect` line"));
                }
                ExpectationKind::Fails(variant) => {
                    self.expected_failure(variant, &fails, &ty);
                }
                ExpectationKind::Performed { cap, op, .. } => self.performed(cap, op),
            }
        }
    }

    /// The own context of a function's body (D-072): its `uses`, declared, or
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
                            clause: (!def.infer_uses).then_some(need.span),
                        },
                    )
                })
                .collect(),
        }];
    }

    /// U3010 and U3011 for a handler declaration (`spec/05 §5`).
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
    /// its capability.
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

    /// `expect fails V`: `V` is a variant of an error kind of the `do` expression, and U1001
    /// otherwise (`spec/10 §12` rule 1, R-020). When the `do` expression cannot fail,
    /// `expect fails V` never holds, and it is U1001 too (ADR 0044). `action` is the type of
    /// the `do` expression: one that failed to type reports nothing more.
    /// D-170 (option B, the owner's answer of 2026-10-01): `fails` is the error kind of the
    /// `do` expression itself, so a call whose failure `attempt` absorbs, or a call inside a
    /// lambda, does not count.
    fn expected_failure(&mut self, variant: &Ident, fails: &[Ty], action: &Ty) {
        let name = self.name(variant);
        if fails.is_empty() {
            if self.resolve(action) == Ty::Error {
                return;
            }
            let mut diagnostic = self.report.make(
                Code::U1001,
                variant.span,
                format!("the `do` expression cannot fail, so `expect fails {name}` never holds"),
            );
            diagnostic.repairs.push(Report::advice(format!(
                "remove the line `expect fails {name}`, or make the `do` line call what fails with `{name}`"
            )));
            self.report.push(diagnostic);
            return;
        }
        let in_kind = fails.iter().any(|kind| match self.resolve(kind) {
            Ty::Kind(id, _) => self.variant_index(id, name).is_some(),
            _ => false,
        });
        if in_kind || fails.iter().any(|kind| self.resolve(kind) == Ty::Error) {
            return;
        }
        let kinds: Vec<String> = fails
            .iter()
            .map(|kind| format!("`{}`", self.show(kind)))
            .collect();
        let message = format!("cannot find variant `{name}` in {}", kinds.join(" or "));
        self.report.error(Code::U1001, variant.span, message);
    }

    /// The position of variant `name` in kind `id`.
    pub(crate) fn variant_index(&self, id: KindId, name: &str) -> Option<usize> {
        self.env
            .kinds
            .get(id)
            .and_then(|kind| kind.variants.iter().position(|v| v.name == name))
    }

    /// `ty`, a type written in the declaration being checked, with the values of its type
    /// parameters labeled with their symbolic labels (ADR 0038 D3.2).
    pub(crate) fn symbolic(&self, ty: &Ty) -> Ty {
        if self.type_params.is_empty() {
            return ty.clone();
        }
        ty.symbolic(&self.param_symbols())
    }

    /// The symbolic label of each type parameter of the declaration being checked: `T`, or
    /// `T*` in edition 0.2 for a parameter constrained by `Ord` or `Integer`, whose type
    /// arguments are scalars, so that every label of one is its top label (ADR 0067 amendment).
    fn param_symbols(&self) -> Vec<String> {
        let def = self.current.and_then(|id| self.env.functions.get(id));
        self.type_params
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let scalar = self.env.edition >= una_project::Edition::V0_2
                    && def.is_some_and(|def| {
                        def.type_params.get(index) == Some(name)
                            && matches!(
                                def.constraints.get(index),
                                Some(Constraint::Ord | Constraint::Integer)
                            )
                    });
                if scalar {
                    format!("{name}*")
                } else {
                    (*name).to_owned()
                }
            })
            .collect()
    }

    /// `ty` without the symbolic labels of the type parameters of the declaration being
    /// checked: the declared result of a generic function may carry them, since each call
    /// replaces them by the labels of its type arguments (`spec/07 §7` rule 8).
    pub(crate) fn without_own_symbols(&self, ty: &Ty) -> Ty {
        if self.type_params.is_empty() {
            return ty.clone();
        }
        let names = self.type_params.clone();
        ty.map_labels(&|label| {
            let mut kept = label.clone();
            kept.symbols
                .retain(|s| !names.iter().any(|n| *n == s.trim_end_matches('*')));
            kept
        })
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

/// The span of a predicate's body, where its value is.
fn predicate_body_span(body: &una_syntax::ast::Body) -> Span {
    match body {
        una_syntax::ast::Body::Line(expr) => expr.span,
        una_syntax::ast::Body::Block(block) => block.span,
    }
}

/// Whether two written types carry the same refinements (ADR 0047 rule 1): the same refined
/// kinds and ranges, in order, and the same refinements of the function types they contain.
/// A range compares by its text, wherever it is written.
/// D-169: ADR 0047 does not say whether an alias without a `where` clause of
/// its own, `kind P2 = Pos`, is another refined kind than `Pos`; it carries exactly the
/// refinements of `Pos`, so the policy call's check against `from` still meets every
/// assumption of the `via` body, and it matches.
fn same_refinements(a: &Declared, b: &Declared) -> bool {
    let same = |x: &Refinement, y: &Refinement| match (x, y) {
        (Refinement::Range(x, _), Refinement::Range(y, _)) => x == y,
        (x, y) => x == y,
    };
    a.refinements.len() == b.refinements.len()
        && a.refinements
            .iter()
            .zip(&b.refinements)
            .all(|(x, y)| same(x, y))
        && !super::call::refinement_flow(&a.base, &b.base)
        && !super::call::refinement_flow(&b.base, &a.base)
}

/// U2026: an operator on values of type parameter `index` of `def` needs the constraint
/// `need`, which its written constraint does not include (`spec/20 §3` rule 1). The repair
/// writes `strongest`, the strongest constraint the body needs of the parameter, as an edit
/// when `edit` (the parameter's first U2026), else as advice: one repair per parameter edits its
/// text (#257).
fn unconstrained(
    report: &Report<'_>,
    def: &FnDef<'_>,
    index: usize,
    need: Constraint,
    span: Span,
    interpolated: bool,
    (strongest, edit): (Constraint, bool),
) -> una_diag::Diagnostic {
    let param = def.type_params.get(index).copied().unwrap_or("T");
    let what = match need {
        Constraint::Integer => "`to_int()`",
        Constraint::Ord => "ordering",
        Constraint::Eq | Constraint::None if interpolated => "interpolation",
        Constraint::Eq | Constraint::None => "equality",
    };
    let need_name = need.name();
    let mut diagnostic = report.make(
        Code::U2026,
        span,
        format!(
            "{what} on values of the type parameter `{param}` needs the constraint `{need_name}`; write `[{param}: {need_name}]`"
        ),
    );
    let strongest_name = strongest.name();
    if !edit {
        diagnostic.repairs.push(Report::advice(format!(
            "constrain `{param}` by `{strongest_name}`, as the first U2026 of `{param}` does"
        )));
    } else if let FnAst::Function(function) = def.ast
        && let Some(ident) = function.type_params.get(index)
    {
        let (at, replacement) = match function.constraints.get(index).and_then(Option::as_ref) {
            Some(written) => (written.span, strongest_name.to_owned()),
            None => (ident.span, format!("{param}: {strongest_name}")),
        };
        diagnostic.repairs.push(report.edit(
            format!("constrain `{param}` by `{strongest_name}`"),
            at,
            replacement,
        ));
    }
    diagnostic
}
