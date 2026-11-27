//! The checker of one declaration at a time: what a position allows, locals, capability
//! bindings, and the rules for each form of declaration.

use std::collections::BTreeMap;

use una_diag::{Code, Escape, ObligationKind, Repair};
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{
    CapBindingValue, CapabilityDecl, ClauseKind, Convention, DeclKind, ExampleDecl,
    ExpectationKind, Expr, FunctionDecl, HandlerCall, HandlerDecl, Ident, OpClauseKind, PolicyDecl,
    PredicateDecl, UsesClause,
};

use super::authority::{Base, Bound, Collected, Frame, Pass};
use super::closest;
use super::crossing::{Install, Node};
use super::env::{Constraint, Entity, Env, FnAst, FnDef, FnKind, HandlerDef};
use super::facts::KindFacts;
use super::label::Label;
use super::records::{Records, Res};
use super::ty::{CapId, Declared, FnUse, KindId, Names, Refinement, Show, Ty};
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
    /// In the condition of an operation of a `uses` clause (ADR 0092), where `it` names no
    /// value.
    pub(crate) op_condition: bool,
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
    /// Inside `e within d`, its timing label `τ(e)` and the `within`: the timing floor that
    /// raises the program counter of every operation and every call with effects in `e`, since
    /// the deadline can cut `e` before any of them (ADR 0117 IF2). Inside a branch of `first
    /// of`, the join of `τ` of every branch and the race, since a sibling that wins can cut the
    /// branch (IF3). The construct is the innermost one that raised the floor.
    pub(crate) timing: Option<(Label, Span, super::within::Construct)>,
}

impl Ctx {
    /// This context with no `result`, `it` or `error` in scope, for the body of a lambda: the
    /// body runs wherever the lambda is called, where none of them names a value, even when
    /// the lambda is written where they do (`spec/02 §8`; U2012, U2013). Both lambda paths,
    /// the plain one and a lambda given for an `of` parameter, start from it.
    pub(crate) fn without_placeholders(&self) -> Self {
        Self {
            result: None,
            it: None,
            op_condition: false,
            error: None,
            ..self.clone()
        }
    }

    pub(crate) fn plain() -> Ctx {
        Ctx {
            result: None,
            ensures: false,
            it: None,
            op_condition: false,
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
            timing: None,
        }
    }
}

/// Where an expression stands when it is not a whole statement and a `let` just before the
/// statement would not keep its meaning (#837, U3013's advice): code that runs only when something
/// else decides, where such a `let` would run a call every time, and a branch of `together`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Conditional {
    /// The value of an `if error use` clause.
    ErrorClause,
    /// The value of an `if none use` clause.
    NoneClause,
    /// A branch of `if ... then ... else`.
    Branch,
    /// The right operand of `and`, `or` or `implies`.
    ShortCircuit,
    /// The one-line body of an arm of `when`.
    Arm,
    /// The value of a branch's `let` in `together`, one `let` per branch (U2024).
    Together,
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
    /// The loop whose variable it is, as `` a `for` loop ``: a loop variable cannot be
    /// declared `edit`, so U2019 on it names a separate `let edit` binding instead (#931).
    pub(crate) loop_of: Option<&'static str>,
}

/// A `for` loop whose body is being checked, for its `stop` and `skip` (`spec/26 §3`, ADR
/// 0116 rule 6, IF2, IF3).
#[derive(Clone, Debug)]
pub(crate) struct LoopFrame {
    /// The program counter its body starts with (IF2).
    pub(crate) pc: Label,
    /// The task and lambda depths of its body: a `stop` or `skip` at a deeper one is U2034.
    pub(crate) depth: (usize, usize),
    /// The program counter that a `skip` under a branch raises the rest of the body to, and
    /// the condition that raised it (IF3).
    pub(crate) skipped: Option<(Label, Option<Span>)>,
    /// The frame is the `on stop` block of its loop, where `stop` and `skip` are U2034.
    pub(crate) on_stop: bool,
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
    /// Whether the lambda about to be checked is an argument of a call of a name that does
    /// not resolve, whose parameters then report nothing (ADR 0096 rule 8).
    pub(crate) unknown_callee_argument: bool,
    /// The innermost lambda is directly an argument of a prelude function, for the advice of
    /// U3004 in edition `0.1`.
    pub(crate) prelude_lambda: bool,
    /// Where the locals of the innermost enclosing `together` task start: a local before it
    /// is captured by the task, which cannot change it (U2023, `spec/19 §2` rule 3).
    pub(crate) task_mark: Option<usize>,
    /// How many `together` tasks enclose the expression checked (`spec/26 §3`).
    pub(crate) task_depth: usize,
    /// What the innermost enclosing task is, for the messages of U2023: a `together` task or
    /// a branch of `first of` (ADR 0117 rule 6).
    pub(crate) task_word: &'static str,
    /// The loops whose bodies enclose the statement checked, innermost last (`spec/26 §3`).
    pub(crate) loops: Vec<LoopFrame>,
    /// Inside an `on stop` block, whose statements must not fail (U3029, `spec/26 §5`).
    pub(crate) in_on_stop: bool,
    /// The innermost code around the expression checked that runs only when something else
    /// decides, so U3013 there cannot advise a `let` before the statement (#837).
    pub(crate) conditional: Option<Conditional>,
    /// Inside the expression of a `start`, which must not fail after its handling clauses
    /// (U3029, `spec/27 §7`).
    pub(crate) in_start: bool,
    /// Inside a branch of `together`, the body of a `together for` or a branch of `first of`:
    /// how a message names it, and the construct's words (ADR 0123 rule 5, U3033).
    pub(crate) server_branch: Option<(&'static str, Span)>,
    /// The `start`s whose expression is being checked, innermost last, with what their captures
    /// can hold (ADR 0123 IF6).
    pub(crate) start_captures: Vec<super::server::CaptureFrame>,
    /// The `tasks` statements whose blocks enclose the statement checked, innermost last: the
    /// task depth and lambda depth at each, where a `start` of it may stand (U2035).
    pub(crate) tasks_frames: Vec<(usize, usize)>,
    /// Inside the body of a `for ever`, which runs no `for ever` and calls no `diverges`
    /// function (U3027, `spec/26 §5`).
    pub(crate) in_ever: bool,
    /// In the `do` line of an example with a `stop after` line, which may run a `for ever`
    /// loop until the stop (U3027, ADR 0116 rule 19).
    pub(crate) do_stops: bool,
    /// The sites whose timing label is being recorded, innermost last (ADR 0116 IF5).
    pub(crate) timing: Vec<usize>,
    /// The task constructs whose parts enclose the code checked, innermost last, by their ids
    /// in `Findings::constructs` (ADR 0116 P2, P3): those of the node being checked only, so
    /// a lambda's body starts with none.
    pub(crate) parts: Vec<usize>,
    /// The `stop` and `skip` statements reported as U2034, after which no U2040 is reported.
    pub(crate) misplaced_exits: Vec<Span>,
    /// The receiver of the method call being checked, with its call-site marker, for its
    /// parameter's convention and exclusivity (`spec/17 §3`).
    pub(crate) receiver_marker: Option<(Option<Convention>, Expr)>,
    /// For the call being checked that passes the elements of a list literal as its arguments
    /// (`spec/29 §17`): the position of the first of them among its arguments, how many there
    /// are, and the parameter the first binds. The next ones bind the parameters after it.
    pub(crate) spread: Option<(usize, usize, usize)>,
    /// The spans of the calls that are the whole expression of a statement, a `let` or an
    /// assignment: the only places for a call with `edit` or `take`, U3013 elsewhere
    /// (`spec/17 §3` rule 5), and where the desugar binds its tuple (`spec/17 §4`).
    pub(crate) statement_calls: Vec<Span>,
    /// The last call with `edit` or `take` reported as U3013 (`spec/17 §3` rule 5): its effects
    /// or failure are not reported again at the same call with other advice (#837).
    pub(crate) marked_nested: Option<Span>,
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
    /// The spans of the receiver and the arguments of a call of a dense kernel being checked,
    /// whose units the kernel's own rule combines (ADR 0120 rule 64): there, a parameter and
    /// an argument that differ only in their units unify, and the argument's type is kept for
    /// that rule.
    pub(crate) unit_blind: BTreeMap<Span, Option<Ty>>,
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
    /// the call: handling such a failure branches on them.
    pub(crate) decided_failures: BTreeMap<Span, Label>,
    /// The policy calls whose `via` function has ADR 0098's shape but a `Keys` handler named
    /// elsewhere keeps from it, by the span of the call: where the handler is named, and why
    /// it matters, for the U4002 that handling the failure gives.
    pub(crate) via_notes: BTreeMap<Span, (Span, String)>,
    /// For each lambda, by its span, which of its parameters are annotated: the body assumes
    /// the annotation, whatever the lambda's type unifies with (#350).
    pub(crate) annotated_lambdas: BTreeMap<Span, Vec<bool>>,
    /// In pass 1, the labels that flow into the label variables of this declaration's editable
    /// bindings, by symbol (ADR 0075).
    pub(crate) edit_bounds: BTreeMap<String, Label>,
    /// In pass 1, the open positions of this declaration's editable bindings: each variable's
    /// name and the unification variable whose type will be its shape (ADR 0075).
    pub(crate) edit_open: Vec<(String, usize)>,
    /// The spans whose unification only propagates an expected type, as a list literal's
    /// items, a branch of a branching expression or a default value: not flows of their own
    /// (ADR 0103), so no `refinement` flow is recorded there.
    pub(crate) flow_hints: std::collections::BTreeSet<Span>,
    /// The outer form of the editable binding being opened, for the example of an annotation.
    pub(crate) edit_outer: String,
    /// The function or predicate whose body is checked.
    pub(crate) current: Option<usize>,
    /// The handler whose operation bodies are checked (ADR 0043).
    pub(crate) handler: Option<usize>,
    /// What the arguments of the calls into boundaries being checked grant them, innermost
    /// last (ADR 0093 rule 5).
    pub(crate) granting: Vec<super::crossing::Granting>,
    /// While a named function given as an argument of a call into a boundary is checked: what
    /// it uses and runs is granted to the boundary (ADR 0093 rule 5).
    pub(crate) granting_direct: bool,
    /// The lambda depth of the body whose own frames `frames` are: 0 for a function's or a
    /// handler's, or a lambda's whose expected type has `uses` (ADR 0093 rule 5).
    pub(crate) frames_depth: usize,
    /// The lambda written at this span whose own frames `frames` are, if any: a binding of its
    /// `uses` is the lambda's, not the enclosing body's (ADR 0093 rule 15).
    pub(crate) frames_lambda: Option<Span>,
    /// The frames that each lambda being checked replaced, innermost last: a binding of a
    /// lambda's frame resolves there, where the lambda is written (ADR 0092 rule 3, #615).
    pub(crate) outer_frames: Vec<Vec<Frame>>,
    /// The bindings of the declaration being checked whose operations its `uses` condition, by
    /// binding and operation (ADR 0092 rule 1, #615).
    pub(crate) conditioned: std::collections::BTreeSet<(String, String)>,
    /// The condition written on each of those operations, by binding and operation (ADR 0092,
    /// amendment of 2026-10-14).
    pub(crate) condition_exprs: std::collections::BTreeMap<(String, String), &'a Expr>,
    /// The function whose `uses` wrote those conditions; `None` in a handler or an example,
    /// where no edit of a callee's condition is computed.
    pub(crate) condition_owner: Option<usize>,
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
    /// In pass 2, the answers of operations that their binding labels (ADR 0094 rule 2): for
    /// each distinct label, the earliest answer in source order that carries it, with the span
    /// of the call, the binding and operation as written there, and the label. The U4001, U4002
    /// and U4003 that such an answer causes name it (rule 7). Its length is bounded by the
    /// distinct `gives` labels of the declaration's header and rebindings, not by its body.
    pub(crate) answers: Vec<super::flow::Answer>,
    /// In pass 2, the labeled arguments of `Json` constructors in edition 0.4 (ADR 0073 J5 as
    /// amended, accepted): for each distinct label, the earliest in source order, which the
    /// U4001, U4002 and U4003 a labeled `Json` causes name (#917).
    pub(crate) json_sources: Vec<super::flow::JsonSource>,
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
    /// The number of frames below its `with` frame: where the right-hand sides of its `with`
    /// resolve, and so where a binding added to that `with` resolves (`spec/05 §3`).
    pub(crate) outer: usize,
}

impl Names for Checker<'_, '_, '_> {
    fn kind_name(&self, id: KindId) -> String {
        self.env.kind_display(id)
    }

    fn param_name(&self, index: usize) -> String {
        self.type_params
            .get(index)
            .map_or_else(|| "?".to_owned(), |name| (*name).to_owned())
    }

    fn refined_name(&self, refinements: &[Refinement], base: &str) -> Option<String> {
        self.env.refined_display(refinements, base)
    }
}

/// A checker's names as source writes them, for repairs.
struct Written<'w, 'c, 'a, 's>(&'w Checker<'c, 'a, 's>);

impl Names for Written<'_, '_, '_, '_> {
    fn kind_name(&self, id: KindId) -> String {
        // A complex kind is written as its type, `Complex[F64]` (ADR 0120 rule 22).
        self.0
            .env
            .complex_written(id)
            .map_or_else(|| self.0.env.kind_name(id).to_owned(), ToOwned::to_owned)
    }

    fn param_name(&self, index: usize) -> String {
        self.0.param_name(index)
    }

    fn refined_name(&self, refinements: &[Refinement], base: &str) -> Option<String> {
        self.0.env.refined_display(refinements, base)
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
    /// The edition the code being checked is checked under: its module's package's (ADR 0122
    /// rule 49), never the root's when a dependency's differs.
    pub(crate) fn edition(&self) -> Edition {
        self.env.edition_of(self.module)
    }

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
            spread: None,
            statement_calls: Vec::new(),
            marked_nested: None,
            inherited: 0,
            of_callee: None,
            of_argument: None,
            of_lambdas: Vec::new(),
            of_found: Vec::new(),
            lambda_depth: 0,
            prelude_argument: false,
            unknown_callee_argument: false,
            prelude_lambda: false,
            task_mark: None,
            task_depth: 0,
            task_word: "`together` task",
            loops: Vec::new(),
            misplaced_exits: Vec::new(),
            in_on_stop: false,
            conditional: None,
            in_start: false,
            server_branch: None,
            start_captures: Vec::new(),
            tasks_frames: Vec::new(),
            in_ever: false,
            do_stops: false,
            timing: Vec::new(),
            parts: Vec::new(),
            call_targets: BTreeMap::new(),
            operands: Vec::new(),
            unit_blind: BTreeMap::new(),
            needs_fields: Vec::new(),
            pending_ops: BTreeMap::new(),
            records: Records::default(),
            decl_span: None,
            ctx: Ctx::plain(),
            returning: None,
            call_fails: BTreeMap::new(),
            decided_failures: BTreeMap::new(),
            via_notes: BTreeMap::new(),
            annotated_lambdas: BTreeMap::new(),
            edit_bounds: BTreeMap::new(),
            edit_open: Vec::new(),
            flow_hints: std::collections::BTreeSet::new(),
            edit_outer: String::new(),
            current: None,
            handler: None,
            granting: Vec::new(),
            granting_direct: false,
            frames_depth: 0,
            frames_lambda: None,
            outer_frames: Vec::new(),
            conditioned: std::collections::BTreeSet::new(),
            condition_exprs: std::collections::BTreeMap::new(),
            condition_owner: None,
            instances: Vec::new(),
            needs: Vec::new(),
            interpolations: Vec::new(),
            written_fields: Vec::new(),
            answers: Vec::new(),
            json_sources: Vec::new(),
            probe_failed: false,
            instantiations: Vec::new(),
            escaped: Vec::new(),
            statement: None,
            absorbed: Vec::new(),
            started,
        }
    }

    // ----- helpers -----

    /// Enters `inner`, code that runs only when something else decides, for U3013's advice
    /// (#837), and returns the context to restore after it. Inside a branch of `together` the
    /// branch's own context stays: the branch is one `let` (U2024), so the inner advice, a
    /// `let`, an `if` on several lines or an arm written as a block, cannot be written there,
    /// and the named function that the branch's advice names can (#2213). A block or a lambda
    /// inside the branch clears it, as it clears every context.
    pub(crate) fn enter_conditional(&mut self, inner: Conditional) -> Option<Conditional> {
        let outer = self.conditional;
        if outer != Some(Conditional::Together) {
            self.conditional = Some(inner);
        }
        outer
    }

    /// The text of a name of this module.
    pub(crate) fn name(&self, ident: &Ident) -> &'a str {
        self.env.text(self.module, ident.name)
    }

    /// A type as written in Una, with solved variables replaced.
    pub(crate) fn show(&self, ty: &Ty) -> String {
        let resolved = self.vars.resolve(ty);
        Show(&resolved, self).to_string()
    }

    /// A type as source writes it, for a repair's text: kinds by their declared names, which
    /// [`Checker::show`] qualifies when two modules share one.
    pub(crate) fn show_written(&self, ty: &Ty) -> String {
        let resolved = self.vars.resolve(ty);
        Show(&resolved, &Written(self)).to_string()
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
        // ADR 0103: the flow, for its pending `refinement` obligation, decided over Core IR
        // once the types are final.
        if !self.flow_hints.contains(&span) {
            self.record_flow(span, expected, found);
        }
        // A dense kernel's operand: its units are the kernel's rule's (ADR 0120 rule 64).
        if let Some(slot) = self.unit_blind.get_mut(&span) {
            *slot = Some(found.clone());
            let (blind_expected, blind_found) = (self.labeled(expected), self.labeled(found));
            if (blind_expected.has_units() || blind_found.has_units())
                && self.vars.unify_expected(
                    &blind_expected.without_units(),
                    &blind_found.without_units(),
                )
            {
                return true;
            }
        }
        if self.vars.unify_expected(expected, found) {
            return true;
        }
        // ADR 0104: two types that differ only in their units are U2028.
        let (resolved_expected, resolved_found) = (self.resolve(expected), self.resolve(found));
        let code = if (resolved_expected.has_units() || resolved_found.has_units())
            && resolved_expected.without_units() == resolved_found.without_units()
        {
            Code::U2028
        } else {
            Code::U2001
        };
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
        self.report.error(code, span, message);
        false
    }

    /// Binds a local name.
    pub(crate) fn bind(&mut self, name: &'a str, ty: Ty) {
        self.bind_as(name, ty, false);
    }

    /// Binds a local name, editable when `editable` (`spec/17 §2`).
    pub(crate) fn bind_as(&mut self, name: &'a str, ty: Ty, editable: bool) {
        self.push_local(name, ty, editable, None);
    }

    /// Binds the variable of `looped`, as `` a `for` loop ``: a plain binding (`spec/17 §1`,
    /// `spec/19 §1`).
    pub(crate) fn bind_loop_var(&mut self, name: &'a str, ty: Ty, looped: &'static str) {
        self.push_local(name, ty, false, Some(looped));
    }

    fn push_local(&mut self, name: &'a str, ty: Ty, editable: bool, loop_of: Option<&'static str>) {
        // ADR 0123 IF6 (#1674): the data the code being checked can hold.
        let label = self.deep(&ty);
        self.reach_label(&label);
        self.locals.push(Local {
            name,
            ty,
            editable,
            taken: None,
            lambda_depth: self.lambda_depth,
            refined: false,
            loop_of,
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

    /// Checks the arguments of a handler instance. Returns the capability it implements, the
    /// handler, and, for a handler of a boundary installed outside it, its installation: its
    /// `uses` are then checked by the caller against the installation's own clause (ADR 0093
    /// rule 3), not here.
    pub(crate) fn install(
        &mut self,
        call: &HandlerCall,
    ) -> (Option<CapId>, Option<usize>, Option<Install>) {
        let name = self.name(&call.name);
        match self.env.lower(self.module, name) {
            Some(Entity::Handler(id)) => {
                self.resolved(call.span, Res::Handler { id });
                self.graph_install(id, call.span);
                if let Some(current) = self.current {
                    self.findings.installs.insert((current, id));
                }
                self.timing_install(id);
                let (params, capability, module, needs) = match self.env.handlers.get(id) {
                    Some(handler) => (
                        handler.params.clone(),
                        handler.capability,
                        handler.module,
                        handler.uses.clone(),
                    ),
                    None => (Vec::new(), None, self.module, Vec::new()),
                };
                let uses: Vec<FnUse> = needs.iter().map(|n| n.usage.clone()).collect();
                let callee = format!("handler `{name}`");
                self.reach(Node::Handler(id));
                // What the arguments give a handler, it may run wherever it is installed (ADR
                // 0093 rule 8). Installing a handler of a boundary is a call into it (rule 3):
                // what its arguments grant also counts against the ceiling.
                let crossing = self.crossing(module);
                let granting = self.granting_here(call.span);
                self.granting.push(granting);
                let checked =
                    self.arguments(&callee, &params, &[], None, &call.args, call.span, module);
                self.argument_flows(&callee, checked);
                // ADR 0117 rule 9: installing `limiter` carries `at_most >= 1`, `per >= 1` and
                // `waiting >= 0`, a `refinement` obligation at each argument (`spec/27 §10`).
                if Some(id) == self.env.limiter {
                    for arg in &call.args {
                        self.obligation(ObligationKind::Refinement, arg.value.span, None);
                    }
                }
                let given = self.granting.pop().unwrap_or_default();
                let install = match crossing {
                    Some(boundary) => Some(Install {
                        boundary,
                        handler: id,
                        module,
                        callee: callee.clone(),
                        name: name.to_owned(),
                        at: call.name.span,
                        uses: needs,
                        span: call.span,
                        granting: given,
                    }),
                    None => {
                        self.handler_arguments(id, call.name.span, given);
                        // The handler's own `uses` are held where it is installed.
                        let level = self.frames.len();
                        self.needs_site(&callee, &uses, call.span, level);
                        // ADR 0092 rule 3: the handler's bindings receive the context's, and
                        // it is charged where it is installed (#615).
                        self.condition_needs(&uses, call.span, level);
                        // ADR 0092, amendment of 2026-10-14: the edit that writes the
                        // context's condition on the handler.
                        self.condition_edits(
                            super::conditions::EditTarget::Handler(id),
                            &call.args,
                            false,
                            call.span,
                            level,
                        );
                        // ADR 0093 rule 15: the handler runs on those bindings wherever it is
                        // run, a boundary it reaches included.
                        let at = call.name.span;
                        let name = name.to_owned();
                        self.installed_needs(id, name, call.span, at, &uses, level, None);
                        None
                    }
                };
                // Its effects run under the program counter of the statement.
                self.call_under_pc(call.span, &callee, &uses, false, true, &Label::public());
                (capability, Some(id), install)
            }
            other => {
                // An unknown name is U1001 (M3); another name is U3010.
                if other.is_some() && self.pass == Pass::Check {
                    self.report.error(
                        Code::U3010,
                        call.name.span,
                        format!("`{name}` is not a handler; `with` and `given` install handlers"),
                    );
                } else if other.is_none() {
                    // An unknown handler (#581): its arguments' inference is not reported.
                    self.vars.mark_unknown();
                }
                for arg in &call.args {
                    self.expr(&arg.value, None);
                }
                (None, None, None)
            }
        }
    }

    /// Whether a `with` binding's value installs `limiter` of `std.limit` (ADR 0117 rule 9).
    pub(crate) fn installs_limiter(&self, value: &CapBindingValue) -> bool {
        match value {
            CapBindingValue::Handler(call) => self.installs_limiter_call(call),
            CapBindingValue::Cap(_) | CapBindingValue::Narrowed { .. } => false,
        }
    }

    /// Whether the handler call `call` names `limiter` of `std.limit`.
    pub(crate) fn installs_limiter_call(&self, call: &HandlerCall) -> bool {
        self.env.limiter.is_some_and(|limiter| {
            matches!(
                self.env.lower(self.module, self.name(&call.name)),
                Some(Entity::Handler(id)) if id == limiter
            )
        })
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

    /// Records, in pass 2, that the call at `span` passes the elements of its argument at
    /// `position`, a list literal, as its arguments (`spec/29 §17`).
    pub(crate) fn spread_at(&mut self, span: Span, position: usize) {
        if self.pass == Pass::Check {
            self.records.spread.insert(span, position);
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

    /// Records, in pass 2, that the type at `span` takes its refinements from the final value
    /// of `from` (ADR 0103 rule 5).
    pub(crate) fn refit(&mut self, span: Span, from: Ty) {
        if self.pass == Pass::Check && self.edition() >= Edition::V0_3 {
            self.records.refits.push((span, from));
        }
    }

    /// Records, in pass 2 and edition `0.3`, that the call at `span` computes new elements
    /// (ADR 0120, `super::dense::SYNTHESIZING`): its flows keep no refinement of the type
    /// arguments a later unification settles, as elaboration does.
    pub(crate) fn synthesized(&mut self, span: Span) {
        if self.pass == Pass::Check && self.edition() >= Edition::V0_3 {
            self.records.synthesized.insert(span);
        }
    }

    /// Records, in pass 2 and edition `0.3`, a value of type `source` at `span` flowing into
    /// `target`, for its pending `refinement` obligation (ADR 0103 rule 4, #615).
    pub(crate) fn record_flow(&mut self, span: Span, target: &Ty, source: &Ty) {
        if self.pass == Pass::Check && self.edition() >= Edition::V0_3 {
            self.records
                .refined_flows
                .push(super::records::RefinedFlow {
                    span,
                    module: self.module,
                    function: self.current,
                    target: target.clone(),
                    source: source.clone(),
                });
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
            module,
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
            // ADR 0100 rule 3: a function with a site of U3007, U3008 or U3009 is poisoned.
            if let Some(current) = current
                && report
                    .diagnostics
                    .get(started..)
                    .is_some_and(|new| new.iter().any(|d| super::dead::poisoning(d.code)))
            {
                findings.family.insert(current);
            }
        }
        for (callee, args, span) in instances {
            let args: Vec<Ty> = args.iter().map(|arg| vars.resolve(arg)).collect();
            findings.instances.push((current, callee, args, span));
        }
        if let Some(current) = current {
            let v0_2 = env.edition_of(module) >= Edition::V0_2;
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
                    // The contracts of the dense methods state the element equations of `get`
                    // and `set` over any element type, which the verifier encodes as equality
                    // of terms of the element's sort (`spec/29 §17`): no constraint is asked of
                    // their callers.
                    if def.module == env.prelude
                        && def.name.starts_with(crate::names::prelude::DENSE_HIDDEN)
                    {
                        continue;
                    }
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
        // An unknown name (#581) leaves its variables unsolved: U1001 is its one diagnostic.
        if report.count() != started || vars.saw_unknown() {
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
        // A declaration without `uses` conditions none; a function and a handler set theirs.
        self.conditioned.clear();
        self.outer_frames.clear();
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
            (DeclKind::Const(constant), Some(Entity::Const(id))) => self.constant(constant, id),
            _ => {}
        }
    }

    /// The conditions of the operations of `uses` (ADR 0092 rule 1): each a `Bool` in the
    /// refinement language over the parameters in scope, with the restrictions of a `requires`
    /// clause (U2001, U2014, U2016, U2012), and no `it` (U2013).
    fn op_conditions(&mut self, uses: &'a una_syntax::ast::UsesClause) {
        for condition in uses.cap_type.conditions.iter().flatten() {
            let ctx = Ctx {
                rl: Rl::Clause,
                labels: false,
                op_condition: true,
                ..Ctx::plain()
            };
            self.within(ctx, |checker| {
                checker.condition(condition, "an operation's condition")
            });
        }
    }

    /// The conditions of a locked clause of `def` (ADR 0092 rule 8, as amended): each a `Bool`
    /// in the refinement language over the function's current parameters, as a header's are,
    /// or U7002 at the lock entry; a lock condition is never ignored. Each one on an operation
    /// that the header grants is a `contract_lock` obligation there.
    fn lock_conditions(&mut self, def: &super::env::FnDef<'a>, entry: &super::lock::Entry<'a>) {
        let ops = &entry.clause.cap_type.ops;
        for (op, condition) in ops.iter().zip(&entry.clause.cap_type.conditions) {
            let Some(condition) = condition else {
                continue;
            };
            let mark = self.report.count();
            let seen = self.vars.swap_unknown(false);
            let ctx = Ctx {
                rl: Rl::Clause,
                labels: false,
                op_condition: true,
                ..Ctx::plain()
            };
            self.within(ctx, |checker| {
                checker.condition(condition, "an operation's condition")
            });
            let unknown = self.vars.swap_unknown(seen);
            let found: Vec<una_diag::Diagnostic> = self.report.diagnostics.split_off(mark);
            let op_name = self.name(op).to_owned();
            if unknown || !found.is_empty() {
                let reason = found.first().map_or_else(
                    || {
                        format!(
                            "it names something that is not a parameter of `{}`",
                            def.name
                        )
                    },
                    |diagnostic| diagnostic.message.clone(),
                );
                self.report.error(
                    self.env.project_code,
                    entry.span,
                    format!(
                        "the lock of `{}` conditions `{op_name}` by `{}`, which is not a `Bool` of the refinement language over its parameters: {reason}",
                        def.name,
                        entry
                            .source
                            .text(condition.span)
                            .unwrap_or_else(|| self.report.text(condition.span))
                    ),
                );
                continue;
            }
            if self.pass == Pass::Check
                && let Some(site) = super::lock::header_site(self.env, def, entry, &op_name)
            {
                self.obligation(ObligationKind::ContractLock, site.at, None);
            }
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
            let ty = self.symbolic(&self.env.top_refined(&param.ty));
            let editable = param.convention == Some(una_syntax::ast::Convention::Edit);
            self.bind_as(param.name, ty, editable);
            if let Some(local) = self.locals.last_mut() {
                local.refined = editable && param.ty.is_refined();
            }
        }
        self.enter_function(&def);
        let written: Vec<&'a UsesClause> = function
            .clauses
            .iter()
            .filter_map(|clause| match &clause.kind {
                ClauseKind::Uses(uses) => Some(uses),
                _ => None,
            })
            .collect();
        self.enter_conditions(&written, Some(id));
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
                ClauseKind::Uses(uses) => self.op_conditions(uses),
                ClauseKind::Fails(_)
                | ClauseKind::UsesOf(_)
                | ClauseKind::FailsOf(_)
                | ClauseKind::Diverges
                | ClauseKind::DerivedFrom(_) => {}
            }
        }
        if let Some(entries) = self.env.locks.get(&id).cloned() {
            for entry in &entries {
                self.lock_conditions(&def, entry);
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
        if !matches!(def.result.base.unlabeled(), Ty::Bool | Ty::Error(_)) {
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
            let mut diagnostic = self.report.make(
                Code::U4004,
                policy.name.span,
                format!(
                    "policy `{}` is declared in module `{path}`, which is not trusted",
                    def.name
                ),
            );
            // ADR 0095: moving the policy into a module that is already trusted keeps the
            // trusted base; declaring this module trusted widens it, so it comes last. A
            // module is trusted only in both places (`spec/07 §6`): one whose header alone
            // says so is U7005, and would only move the error.
            let trusted: Vec<String> = self
                .env
                .modules
                .iter()
                .take(self.env.prelude)
                .filter(|m| {
                    m.tree.module.as_ref().is_some_and(|header| header.trusted)
                        && self.env.trusted_listed.contains(&m.path)
                })
                .map(|m| format!("`{}`", m.path))
                .collect();
            if !trusted.is_empty() {
                diagnostic.repairs.push(super::Report::advice(format!(
                    "move the policy and its `via` function into a module that is already trusted: {}",
                    trusted.join(", ")
                )));
            }
            diagnostic.repairs.push(super::Report::escape(
                Escape::Trusted,
                format!(
                    "declare `trusted module {path}` in its header and list `{path}` under `[trusted] modules` in `una.toml`"
                ),
            ));
            self.report.push(diagnostic);
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
            .map_or_else(|| Declared::plain(Ty::error()), |p| p.ty.clone());
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
                            gives: need.usage.gives.clone(),
                        },
                    )
                })
                .collect(),
        }];
        self.handler_rules(handler, &def);
        let written: Vec<&'a UsesClause> = handler.uses.iter().collect();
        self.enter_conditions(&written, None);
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
        // A handler `for` a name that does not resolve (#581): its operations' signatures are
        // unknown, so what inference leaves unsolved in their bodies is not reported.
        if capability.is_none() {
            self.vars.mark_unknown();
        }
        // The conditions of the handler's own `uses`, over its parameters (ADR 0092 rule 1).
        if handler
            .uses
            .iter()
            .any(|uses| uses.cap_type.conditions.iter().any(Option::is_some))
        {
            let mark = self.locals.len();
            for param in &def.params {
                self.bind(param.name, param.ty.base.clone());
            }
            let base = std::mem::replace(&mut self.base, Base::Quiet);
            for uses in &handler.uses {
                self.op_conditions(uses);
            }
            self.base = base;
            self.locals.truncate(mark);
        }
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
                    .map_or(Ty::error(), |p| p.ty.base.raise_deep(raise));
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
        // An example has no `uses` of its own, so nothing it does is conditioned (#615).
        self.enter_conditions(&[], None);
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
            let (capability, handler, install) = self.install(&given.handler);
            if let Some(install) = install {
                // ADR 0093 rule 3: the `given` lines before it are the clause that grants a
                // boundary handler's `uses`, as they hold any handler's (`spec/05 §9` rule 4).
                let level = self.frames.len();
                let needs: Vec<FnUse> = install.uses.iter().map(|n| n.usage.clone()).collect();
                self.needs_site(&install.callee, &needs, install.span, level);
                // ADR 0093 rule 15: another boundary may receive it by name from here.
                self.record_installed(install.handler, install.at, &install.granting);
                self.installed_needs(
                    install.handler,
                    install.name.clone(),
                    install.span,
                    install.at,
                    &needs,
                    level,
                    Some(install.boundary),
                );
                let (boundary, callee, span) =
                    (install.boundary, install.callee.clone(), install.span);
                self.cross(
                    boundary,
                    &format!("the installation of {callee}"),
                    Node::Handler(install.handler),
                    &needs,
                    level,
                    install.granting,
                    span,
                );
            }
            if let Some(handler) = handler {
                // ADR 0093 rule 10: an example of a boundary module grants it its `given`s.
                let handler_name = self.name(&given.handler.name);
                self.given_in_boundary(handler, handler_name, given.handler.span);
            }
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
                        handler,
                    },
                ));
            }
        }
        self.ctx.in_do = true;
        self.do_stops = example.stop_after.is_some();
        self.absorbed.clear();
        // ADR 0117 rule 9, IF5: a `given` line that installs `limiter` of `std.limit` makes the
        // `do` line a site, whose seed covers the handlers the `given` lines install.
        let limiters: Vec<Span> = example
            .givens
            .iter()
            .filter(|given| self.installs_limiter_call(&given.handler))
            .map(|given| given.handler.span)
            .collect();
        if !limiters.is_empty() {
            self.timing_start();
            // The `do` line is the root of the test's run: placed, as `main` is.
            self.timing_placed();
            for given in &example.givens {
                if let Some(Entity::Handler(id)) =
                    self.env.lower(self.module, self.name(&given.handler.name))
                {
                    self.timing_install(id);
                }
            }
        }
        let ty = self.expr(&example.action, None);
        if !limiters.is_empty() {
            let seed = self.timing_end();
            if self.pass == Pass::Check {
                for span in limiters {
                    self.findings.limiter_sites.push(super::LimiterSite {
                        span,
                        function: None,
                        frame: seed,
                    });
                }
            }
        }
        self.ctx.in_do = false;
        self.do_stops = false;
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
                            gives: need.usage.gives.clone(),
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
                        format!(
                            "capability `{}` has no operation `{name}`",
                            self.env.cap_def_name(&capability)
                        ),
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
    /// its capability. An operation the capability does not declare is U1001, with the message
    /// and the closest-operation edit of a call in a body (ADR 0082).
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
                    let mut diagnostic = self.report.make(
                        Code::U1001,
                        op.span,
                        format!(
                            "capability `{}` has no operation `{op_name}`",
                            self.env.cap_def_name(capability)
                        ),
                    );
                    if let Some(close) = closest(op_name, capability.ops.iter().map(|o| o.name)) {
                        diagnostic.repairs.push(self.report.edit(
                            format!("write `{close}`"),
                            op.span,
                            close,
                        ));
                    }
                    self.report.push(diagnostic);
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
    /// D-170 (option B, decided on 2026-10-01): `fails` is the error kind of the
    /// `do` expression itself, so a call whose failure `attempt` absorbs, or a call inside a
    /// lambda, does not count.
    fn expected_failure(&mut self, variant: &Ident, fails: &[Ty], action: &Ty) {
        let name = self.name(variant);
        if fails.is_empty() {
            if self.resolve(action) == Ty::error() {
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
        if in_kind || fails.iter().any(|kind| self.resolve(kind) == Ty::error()) {
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
                let scalar = self.edition() >= una_project::Edition::V0_2
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
