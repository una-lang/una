//! Information flow (M6): the program counter, flows into declared types (U4001), failures and
//! calls under the program counter (U4002), sinks (U4003), following `spec/07`, the label
//! premises of `spec/10` and ADR 0031.

use una_diag::{Code, Escape};
use una_syntax::Span;

use super::Report;
use super::authority::{Base, Pass};
use super::check::Checker;
use super::label::{Accepts, Label};
use super::ty::{FnUse, KindLabels, ShowLabeled, Ty};

/// `from ⊑ to`, where a symbolic label is below `to` only if `to` contains it or is `⊤`
/// (`spec/07 §7` rule 8, ADR 0038 D3.2).
pub(crate) fn below(from: &Label, to: &Label, top: &Label) -> bool {
    // `ℓ_T`, the label of a value of `T`, is below `ℓ*_T`, every label of its type argument.
    let mut from = from.clone();
    from.symbols
        .retain(|s| s.ends_with('*') || !to.symbols.contains(&format!("{s}*")));
    let from = &from;
    if from.flows_to(to) {
        return true;
    }
    top.concrete().flows_to(to) && from.concrete().flows_to(to)
}

/// The first position where a value's type does not flow into a declared type: the label
/// found there and the label allowed there. `top` is `⊤`. Kind arguments are compared by the
/// variance of their parameters (ADR 0046 rule 2): a covariant one as data, an invariant one
/// in both directions, so that its labels are equal at every position.
/// The third element says whether the position is covariant, where the value's data flows
/// into the declared type, rather than contravariant, where the declared type's flows into it.
fn violation(
    found: &Ty,
    expected: &Ty,
    covariant: bool,
    top: &Label,
    kinds: &KindLabels,
) -> Option<(Label, Label, bool)> {
    // A `Unit` value has one inhabitant, so it carries no information.
    if matches!(expected.unlabeled(), Ty::Unit) {
        return None;
    }
    let (from, to) = if covariant {
        (found.top_label(), expected.top_label())
    } else {
        (expected.top_label(), found.top_label())
    };
    if !below(&from, &to, top) {
        return Some((from, to, covariant));
    }
    match (found.unlabeled(), expected.unlabeled()) {
        (Ty::List(x), Ty::List(y)) | (Ty::Dense(_, x), Ty::Dense(_, y)) => {
            violation(x, y, covariant, top, kinds)
        }
        (Ty::Map(xk, xv), Ty::Map(yk, yv)) => violation(xk, yk, covariant, top, kinds)
            .or_else(|| violation(xv, yv, covariant, top, kinds)),
        // A collection is a value, as a map is: its arguments are compared as data.
        (Ty::Coll(i, xs), Ty::Coll(j, ys)) if i == j => xs
            .iter()
            .zip(ys.iter())
            .find_map(|(x, y)| violation(x, y, covariant, top, kinds)),
        (Ty::Kind(i, xs), Ty::Kind(j, ys)) if i == j => {
            xs.iter().zip(ys).enumerate().find_map(|(index, (x, y))| {
                violation(x, y, covariant, top, kinds).or_else(|| {
                    if kinds.is_invariant(*i, index) {
                        violation(x, y, !covariant, top, kinds)
                    } else {
                        None
                    }
                })
            })
        }
        (Ty::Fn(f), Ty::Fn(g)) => f
            .params
            .iter()
            .zip(&g.params)
            .find_map(|(x, y)| violation(x, y, !covariant, top, kinds))
            .or_else(|| violation(&f.result, &g.result, covariant, top, kinds)),
        _ => None,
    }
}

/// The answer of an operation that its binding labels (ADR 0094 rule 2), where it is called.
#[derive(Clone, Debug)]
pub(crate) struct Answer {
    /// The call.
    pub(crate) span: Span,
    /// The binding, without `@`, as the call writes it.
    pub(crate) binding: String,
    /// The operation.
    pub(crate) op: String,
    /// The label the binding gives the operation's answers.
    pub(crate) label: Label,
}

/// What carries a labeled answer into a flow, for the words of ADR 0094 rule 7: a value, or
/// the program counter that a branch on the answer, or on whether it failed, raised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Carrier {
    /// The value at the flow, `this value`, `this argument` or `this item`.
    Value(&'static str),
    /// The condition the code runs under.
    Condition,
    /// A parameter of a function value, at a contravariant position of a flow: the answer
    /// would flow into it.
    Parameter,
}

/// Whether `label` contains the symbolic label of a type parameter.
pub(crate) fn symbolic(label: &Label) -> bool {
    label.symbols.iter().any(|symbol| !symbol.starts_with('?'))
}

impl Checker<'_, '_, '_> {
    /// Whether labels are checked here: in pass 2, outside clauses, `via` bodies and `expect`
    /// lines, whose contexts turn them off.
    pub(crate) fn labels(&self) -> bool {
        self.pass == Pass::Check && self.ctx.labels
    }

    /// Whether code here stands under a program counter above `public` for the trace (S-39,
    /// `spec/26 §6` rule 4): the events that the runtime appends by itself, `stop`, `timeout`
    /// and `cancelled`, are sinks that accept `public`, as `Console.print` is. The program
    /// counter counts with the effect floor of an operation body and the timing floor of a
    /// `within`, as an effect's does (`spec/07 §3` rule 4, `spec/27 §5`), and a body that no
    /// label rule checks counts as `⊤`. A label of a type parameter is not `public` either.
    fn raised(&self) -> bool {
        let mut pc = self.ctx.pc.clone();
        if let Some(floor) = &self.ctx.floor {
            pc = pc.join(floor);
        }
        if let Some((timing, _, _)) = &self.ctx.timing {
            pc = pc.join(timing);
        }
        !self.ctx.labels || !pc.is_public()
    }

    /// Records `span`, a construct whose running appends events of its own to the trace, as
    /// `quiet` when it stands under a raised program counter ([`Self::raised`]): a `for ever`
    /// loop, a `within`, a `first of`, a `tasks`, `together` or `together for` statement, a call
    /// of a function value, or an operation, which a handler of the package may serve (S-39,
    /// `spec/26 §6` rule 4).
    pub(crate) fn quiet_site(&mut self, span: Span) {
        self.quiet_site_under(span, &Label::public());
    }

    /// As [`Self::quiet_site`], for a call that stands under `pc ⊔ raise` rather than `pc`: the
    /// call of a function value labeled `l_f` stands under `pc ⊔ l_f` (`spec/07 §3` rule 4,
    /// T-App), and the value's label is joined with the decisions its arguments carry (ADR 0067
    /// amendment). Such a call is quiet when `raise` is above `public`, a symbolic label
    /// included, even at a `public` program counter (S-39 round 4, `spec/26 §6` rule 4).
    pub(crate) fn quiet_site_under(&mut self, span: Span, raise: &Label) {
        if self.pass == Pass::Check && (self.raised() || !raise.is_public()) {
            self.findings.quiet.insert(span);
        }
    }

    /// Records the call at `span` of the function `callee` under a raised program counter
    /// ([`Self::raised`]): once every body is checked, [`close_quiet`] marks it `quiet` when the
    /// callee can reach a construct whose running appends events of its own.
    pub(crate) fn quiet_call_site(&mut self, span: Span, callee: usize) {
        self.quiet_call_site_under(span, callee, &Label::public());
    }

    /// As [`Self::quiet_call_site`], for a named call that runs under `pc ⊔ raise`: the
    /// decision that edition 0.1 raises a call by when its arguments give function values data
    /// decided by values of a type parameter (`ℓ*_T`, T-Call, #346, #350).
    pub(crate) fn quiet_call_site_under(&mut self, span: Span, callee: usize, raise: &Label) {
        if self.pass == Pass::Check && (self.raised() || !raise.is_public()) {
            self.findings.quiet_calls.insert(span, callee);
        }
    }

    /// Records the operation at `span` under a raised program counter ([`Self::raised`]): once
    /// every body is checked, [`close_quiet`] marks it `quiet` when the package declares a
    /// handler, whose operation bodies may run such a construct.
    pub(crate) fn quiet_op_site(&mut self, span: Span) {
        if self.pass == Pass::Check && self.raised() {
            self.findings.quiet_ops.insert(span);
        }
    }

    /// `ty` with its solved variables replaced, labels included.
    pub(crate) fn labeled(&self, ty: &Ty) -> Ty {
        self.vars.resolve_labeled(ty)
    }

    /// The deep label of `ty`, its solved variables replaced: the join of every label at a
    /// position that holds data, the labels its kinds' fields declare included (`spec/07 §7`
    /// rule 3).
    pub(crate) fn deep(&self, ty: &Ty) -> Label {
        self.labeled(ty).deep_label(&self.env.kind_labels)
    }

    /// The first answer checked in this declaration, in source order, that the binding labels
    /// above `allowed` and whose label `found` carries: the labeled answer a flow from `found`
    /// to `allowed` reveals (ADR 0094 rule 7). Every answer the flow carries has its label
    /// within `found`, so the answer that caused the flow is never filtered out; when several
    /// qualify, the first in source order is named.
    ///
    /// [`Self::note_answer`] keeps one answer per distinct label, so the scan is bounded by the
    /// labels the declaration's header and rebindings write, not by the size of its body, and
    /// reporting many flows in a body that reads many answers stays linear.
    pub(crate) fn answer_in(&self, found: &Label, allowed: &Label) -> Option<&Answer> {
        let top = self.top();
        self.answers
            .iter()
            .filter(|answer| {
                below(&answer.label, found, &top) && !below(&answer.label, allowed, &top)
            })
            .min_by_key(|answer| (answer.span.start, answer.span.end))
    }

    /// Records a labeled answer (ADR 0094 rule 7): the earliest in source order of each
    /// distinct label is kept, since [`Self::answer_in`] chooses by label and then by source
    /// order, so the choice is the same as over every answer.
    pub(crate) fn note_answer(&mut self, answer: Answer) {
        let key = |a: &Answer| (a.span.start, a.span.end);
        match self
            .answers
            .iter_mut()
            .find(|known| known.label == answer.label)
        {
            Some(known) if key(&answer) < key(known) => *known = answer,
            Some(_) => {}
            None => self.answers.push(answer),
        }
    }

    /// Names in `diagnostic`, a U4001, U4002 or U4003 of a flow from `found` to `allowed`,
    /// the labeled answer that the flow reveals, when there is one ([`Self::answer_in`]): the
    /// message says that `carrier` carries the answer of the binding's operation and its label,
    /// the call is a related location, and the first repair names them too, with today's
    /// routes: read the operation through another binding, change the sink, or, last, release
    /// the answer through a policy of a trusted module (ADR 0094 rule 7, ADR 0095).
    pub(crate) fn name_answer(
        &self,
        diagnostic: &mut una_diag::Diagnostic,
        carrier: Carrier,
        found: &Label,
        allowed: &Label,
    ) {
        let Some(answer) = self.answer_in(found, allowed) else {
            return;
        };
        let (binding, op, label) = (&answer.binding, &answer.op, &answer.label);
        let named = format!("the answer of `@{binding}.{op}`, labeled `{label}`");
        let (subject, route) = match carrier {
            Carrier::Value(what) => (
                what,
                format!("or send the value only where `{label}` data may go"),
            ),
            Carrier::Condition => (
                "the condition",
                "or decide this without branching on it".to_owned(),
            ),
            Carrier::Parameter => (
                "this flow",
                format!("or pass a function whose parameter accepts `{label}` data"),
            ),
        };
        diagnostic
            .message
            .push_str(&format!("; {subject} carries {named}"));
        // The call as a related location, unless the diagnostic is there or already names it.
        let at = self.report.location(answer.span);
        if at != diagnostic.primary && diagnostic.related.iter().all(|r| r.location != at) {
            diagnostic.related.push(
                self.report
                    .related(answer.span, format!("the answer of `@{binding}.{op}`")),
            );
        }
        diagnostic.repairs.insert(
            0,
            Report::advice(format!(
                "{subject} carries {named}: read `{op}` through a binding whose `{op}` answers within `{allowed}`, {route}"
            )),
        );
        if !diagnostic
            .repairs
            .iter()
            .any(|repair| repair.escape.is_some())
        {
            diagnostic.repairs.push(Report::escape(
                Escape::Trusted,
                format!(
                    "release the answer of `@{binding}.{op}` through a policy of a trusted module"
                ),
            ));
        }
    }

    /// Reports `code` at `span` with `message`, a flow from `found` to `allowed`, naming the
    /// labeled answer that `carrier` carries into it (ADR 0094 rule 7).
    pub(crate) fn flow_error(
        &mut self,
        code: Code,
        span: Span,
        message: String,
        carrier: Carrier,
        found: &Label,
        allowed: &Label,
    ) {
        let mut diagnostic = self.report.make(code, span, message);
        self.name_answer(&mut diagnostic, carrier, found, allowed);
        self.report.push(diagnostic);
    }

    /// `⊤`: `restricted` with every declared category.
    pub(crate) fn top(&self) -> Label {
        Label::top(&self.env.categories)
    }

    /// U4001: the value at `span`, of type `found`, flows into `expected`, a declared type
    /// described by `into` (`spec/07 §3` rule 5, `spec/10 §2`).
    pub(crate) fn flow(&mut self, span: Span, found: &Ty, expected: &Ty, into: &str) {
        self.flow_as(span, found, expected, None, into, None);
    }

    /// [`Checker::flow`] into a declared type that the program cannot change, a field of a
    /// kind of the standard library or a parameter of a host operation, with `advice` on
    /// what can apply instead of raising it (#404).
    pub(crate) fn flow_fixed(
        &mut self,
        span: Span,
        found: &Ty,
        expected: &Ty,
        into: &str,
        advice: String,
    ) {
        self.flow_as(span, found, expected, None, into, Some(advice));
    }

    /// U4001: an argument at `span`, of type `found`, of an operation whose parameter, described
    /// by `into`, is declared `declared` (ADR 0050, `spec/07 §7` rule 4). It flows into the
    /// declared type with every covariant position taken as `⊤`, since `accepts` bounds the
    /// data there ([`Checker::sink`]); contravariant positions and invariant kind arguments are
    /// compared as at any other call argument.
    pub(crate) fn op_argument_flow(
        &mut self,
        span: Span,
        found: &Ty,
        declared: &Ty,
        into: &str,
        advice: Option<String>,
    ) {
        if !self.labels() {
            return;
        }
        let view = self
            .labeled(declared)
            .op_parameter(true, &self.top(), &self.env.kind_labels);
        self.flow_as(span, found, &view, Some(declared), into, advice);
    }

    /// [`Checker::flow`] into `expected`; the message shows `shown` instead when given, and
    /// `advice` is offered as a repair without an edit.
    fn flow_as(
        &mut self,
        span: Span,
        found: &Ty,
        expected: &Ty,
        shown: Option<&Ty>,
        into: &str,
        advice: Option<String>,
    ) {
        if !self.labels() {
            return;
        }
        let (found_ty, expected_ty) = (self.labeled(found), self.labeled(expected));
        let top = self.top();
        let Some((from, to, covariant)) =
            violation(&found_ty, &expected_ty, true, &top, &self.env.kind_labels)
        else {
            return;
        };
        let expected_ty = shown.map_or(expected_ty, |shown| self.labeled(shown));
        let (found_shown, expected_shown) = (
            ShowLabeled(&found_ty, self).to_string(),
            ShowLabeled(&expected_ty, self).to_string(),
        );
        let residue = self.residue_clause_for(&from, &to);
        let mut diagnostic = self.report.make(
            Code::U4001,
            span,
            format!(
                "{into} is `{expected_shown}`, and this value is `{found_shown}`: a flow from `{from}` to `{to}`{residue}"
            ),
        );
        if let Some(advice) = advice {
            diagnostic.repairs.push(Report::advice(advice));
        }
        // At a contravariant position, a parameter of a function type, the data flows from the
        // declared type into the value: the answer would reach the value's parameter.
        let carrier = if covariant {
            Carrier::Value("this value")
        } else {
            Carrier::Parameter
        };
        self.name_answer(&mut diagnostic, carrier, &from, &to);
        self.report.push(diagnostic);
    }

    /// U4001: each branch of a construct, at its span and of its type, flows into the value
    /// that joins them (ADR 0046 rule 3). The join is above every branch except at an
    /// invariant parameter of a kind, where it keeps the labels of the first branch.
    /// D-160: ADR 0046 rule 3 makes a branch join a flow site without saying which
    /// type the join has; it is the first branch's at an invariant argument, so a later
    /// branch that differs there is U4001 at that branch.
    pub(crate) fn join_flows(&mut self, joined: &Ty, branches: Vec<(Span, Ty)>) {
        if branches.len() < 2 {
            return;
        }
        for (span, ty) in branches {
            self.flow(span, &ty, joined, "the value that joins the branches");
        }
    }

    /// U4001 for an implicit flow into a result: a branching construct in a return position
    /// whose condition, labeled `label`, exceeds the result's top label.
    pub(crate) fn implicit_result(
        &mut self,
        condition: Span,
        label: &Label,
        expected: &Ty,
        into: &str,
    ) {
        if !self.labels() {
            return;
        }
        let expected = self.labeled(expected);
        if matches!(expected.unlabeled(), Ty::Unit) {
            return;
        }
        let allowed = expected.top_label();
        if below(label, &allowed, &self.top()) {
            return;
        }
        let mut diagnostic = self.report.make(
            Code::U4001,
            condition,
            format!(
                "{into} is `{allowed}`, and which value it returns depends on this `{label}` condition: a flow from `{label}` to `{allowed}`"
            ),
        );
        self.name_answer(&mut diagnostic, Carrier::Condition, label, &allowed);
        self.report.push(diagnostic);
    }

    /// The program counter inside the branches of a construct whose condition at `span` has
    /// label `label`, and the value raise; returns the saved program counter.
    pub(crate) fn raise_pc(&mut self, label: &Label, span: Span) -> (Label, Option<Span>) {
        let saved = (self.ctx.pc.clone(), self.ctx.pc_from);
        if !label.is_public() {
            self.ctx.pc = self.ctx.pc.join(label);
            self.ctx.pc_from = Some(span);
        }
        saved
    }

    /// Restores the program counter that [`Checker::raise_pc`] saved.
    pub(crate) fn restore_pc(&mut self, saved: (Label, Option<Span>)) {
        self.ctx.pc = saved.0;
        self.ctx.pc_from = saved.1;
    }

    /// U4002: `what` fails, which is allowed only under a `public` program counter
    /// (`spec/07 §3` rule 3). `extra` is the label of a value it also branches on.
    /// At the construct, with the labeled condition as a related location.
    pub(crate) fn fail_under_pc(&mut self, span: Span, what: &str, extra: Option<(&Label, Span)>) {
        if !self.labels() {
            return;
        }
        let mut label = self.ctx.pc.clone();
        let mut from = self.ctx.pc_from;
        if let Some((value, value_span)) = extra
            && !value.is_public()
        {
            label = label.join(value);
            from = from.or(Some(value_span));
        }
        if label.is_public() {
            return;
        }
        let mut diagnostic = self.report.make(
            Code::U4002,
            span,
            format!(
                "{what} depends on `{label}` data, so its failure would reveal it; return a labeled value instead of failing"
            ),
        );
        if let Some(from) = from
            && from != span
        {
            diagnostic
                .related
                .push(self.report.related(from, "the labeled condition"));
        }
        // ADR 0098: a policy call whose `via` would fail publicly but for a `Keys` handler
        // named elsewhere says where, so the reason is not hidden.
        if let Some((_, call)) = extra
            && let Some((at, note)) = self.via_notes.get(&call)
        {
            diagnostic
                .related
                .push(self.report.related(*at, note.clone()));
        }
        self.name_answer(
            &mut diagnostic,
            Carrier::Condition,
            &label,
            &Label::public(),
        );
        self.report.push(diagnostic);
    }

    /// The code checked, for the highest program counter it can be entered under (S-14,
    /// ADR 0085).
    pub(crate) fn entry(&self) -> super::env::Entry {
        use super::env::Entry;
        if self.lambda_depth > 0 {
            Entry::Lambda
        } else if let Some(floor) = &self.ctx.floor {
            Entry::Floor(floor.clone())
        } else if let Some(f) = self.current {
            Entry::Function(f)
        } else {
            Entry::Root
        }
    }

    /// The meet of the `accepts` labels of the operations in `uses`, `bound_eff` of
    /// `spec/07 §7` rule 4, and `⊤` (`None`) when none of them bounds its arguments
    /// (`spec/10 §5`). A callee that can fail is bounded by `⊥` besides, which
    /// [`Checker::call_under_pc`] checks at the program counter of the call.
    pub(crate) fn bound(&self, uses: &[FnUse]) -> Option<Label> {
        bound_of(self.env, uses)
    }

    /// U4002: a call of `callee`, or the installation of a handler (`installed`), with the
    /// effects of `uses` and failures when `fails`, under the program counter (`spec/07 §3`
    /// rule 4, `spec/10 §5`). `value` is the label of the function value called,
    /// which raises the program counter of the call (T-App, ADR 0038 D1).
    /// - A callee that can fail is allowed only under a `public` program counter, that of the
    ///   body itself in a handler operation (ADR 0038 D4).
    /// - Effects are allowed only when the program counter, raised by the effect floor of a
    ///   handler operation body (ADR 0038 D4), is below `bound_eff`, and never under a
    ///   symbolic program counter (ADR 0038 D3.3).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call_under_pc(
        &mut self,
        span: Span,
        callee: &str,
        uses: &[FnUse],
        fails: bool,
        installed: bool,
        value: &Label,
    ) {
        if !self.labels() {
            return;
        }
        let pc = self.ctx.pc.join(value);
        let a = article(&pc.to_string());
        let action = if installed { "installed" } else { "called" };
        if fails && !pc.is_public() {
            self.under_pc(
                span,
                format!(
                    "{callee} can fail, and it is {action} under {a} `{pc}` condition, which would be revealed"
                ),
                Some((&pc, &Label::public())),
            );
            return;
        }
        if uses.is_empty() {
            return;
        }
        let floor = self.ctx.floor.clone();
        let effect_pc = floor.as_ref().map_or_else(|| pc.clone(), |f| pc.join(f));
        // ADR 0117 IF2: inside `e within d`, the timing floor raises it too.
        let timing = self.ctx.timing.clone();
        let effect_pc = timing
            .as_ref()
            .map_or_else(|| effect_pc.clone(), |(t, _, _)| effect_pc.join(t));
        if effect_pc.is_public() {
            return;
        }
        if symbolic(&effect_pc) {
            self.under_pc(
                span,
                format!(
                    "{callee} has effects, and it is {action} under {a} `{pc}` condition on a value of a type parameter, whose label is known only at each call"
                ),
                None,
            );
            return;
        }
        let Some(bound) = self.bound(uses) else {
            return;
        };
        if effect_pc.flows_to(&bound) {
            return;
        }
        let floored = floor.as_ref().map_or_else(|| pc.clone(), |f| pc.join(f));
        if let Some((t, within, construct)) = timing
            && floored.flows_to(&bound)
        {
            let place = match construct {
                super::within::Construct::Within => "inside a `within` whose deadline can cut it",
                super::within::Construct::FirstOf => {
                    "in a branch of `first of`, which a sibling that wins can cut"
                }
            };
            let words = construct.words();
            let residue = self.residue_clause(&bound);
            let mut diagnostic = self.report.make(
                Code::U4002,
                span,
                format!(
                    "{callee} has effects that accept at most `{bound}` data, and it is {action} {place}, depending on `{t}` timing, which whether its effects happen would reveal{residue}"
                ),
            );
            diagnostic
                .related
                .push(self.report.related(within, format!("the `{words}`")));
            self.report.push(diagnostic);
            return;
        }
        let message = match floor {
            Some(floor) if pc.flows_to(&bound) => format!(
                "{callee} has effects that accept at most `{bound}` data, and it is {action} in an operation body whose callers may pass `{floor}` data, which it would reveal"
            ),
            _ => format!(
                "{callee} has effects that accept at most `{bound}` data, and it is {action} under {a} `{pc}` condition, which would be revealed"
            ),
        };
        self.under_pc(span, message, Some((&pc, &bound)));
    }

    /// U4002 at `span`, with the condition that raised the program counter as a related
    /// location. `flow`, the program counter's label and the label it exceeds, names the
    /// labeled answer the condition carries (ADR 0094 rule 7).
    fn under_pc(&mut self, span: Span, message: String, flow: Option<(&Label, &Label)>) {
        let mut diagnostic = self.report.make(Code::U4002, span, message);
        if let Some(from) = self.ctx.pc_from {
            diagnostic
                .related
                .push(self.report.related(from, "the labeled condition"));
        }
        if let Some((found, allowed)) = flow {
            self.name_answer(&mut diagnostic, Carrier::Condition, found, allowed);
        }
        self.report.push(diagnostic);
    }
    /// U4002: a policy that lowers data to `to`, called under a program counter above it
    /// (`spec/07 §5` rule 2, T-Declassify).
    pub(crate) fn declassify_under_pc(&mut self, span: Span, callee: &str, to: &Label) {
        if !self.labels() || self.ctx.pc.flows_to(to) {
            return;
        }
        let pc = self.ctx.pc.clone();
        let a = article(&pc.to_string());
        let mut diagnostic = self.report.make(
            Code::U4002,
            span,
            format!(
                "{callee} lowers data to `{to}`, and it is called under {a} `{pc}` condition, which its result would reveal"
            ),
        );
        if let Some(from) = self.ctx.pc_from {
            diagnostic
                .related
                .push(self.report.related(from, "the labeled condition"));
        }
        self.name_answer(&mut diagnostic, Carrier::Condition, &pc, to);
        self.report.push(diagnostic);
    }

    /// U4005: the `via` function `id` of a policy is used at `span` other than through its
    /// policy, which is its only entry point (`spec/07 §5` rule 3).
    pub(crate) fn via_site(&mut self, id: usize, span: Span) {
        // A `via` function may call itself.
        if self.pass != Pass::Check || self.base == Base::Quiet || self.current == Some(id) {
            return;
        }
        let Some(policy) = self.env.vias.get(&id).copied() else {
            return;
        };
        let (name, policy_name) = (
            self.env.functions.get(id).map_or("?", |f| f.name),
            self.env.functions.get(policy).map_or("?", |f| f.name),
        );
        self.report.error(
            Code::U4005,
            span,
            format!(
                "`{name}` is the `via` function of policy `{policy_name}`, which is its only entry point; call `{policy_name}` instead"
            ),
        );
    }

    /// U4003: operation `what`, whose arguments are sinks with label `accepts`, under the
    /// program counter, with `args` (`spec/07 §3` rule 4, `spec/07 §4`).
    /// At the argument that exceeds `accepts`, or at the operation when the
    /// program counter alone exceeds it.
    pub(crate) fn sink(&mut self, span: Span, what: &str, accepts: &Accepts, args: &[(Span, Ty)]) {
        if !self.labels() {
            return;
        }
        // ADR 0038 D3.3: no capability operation under a symbolic program counter.
        if symbolic(&self.ctx.pc) {
            let pc = self.ctx.pc.clone();
            let a = article(&pc.to_string());
            self.under_pc(
                span,
                format!(
                    "{what} runs under {a} `{pc}` condition on a value of a type parameter, whose label is known only at each call, so its sink cannot be checked here"
                ),
                None,
            );
            return;
        }
        let Some(accepts) = accepts else {
            return;
        };
        // ADR 0038 D4: in a handler operation body, the effect floor raises the program
        // counter to the operation's `accepts`.
        let floor = self.ctx.floor.clone();
        let floored = floor
            .as_ref()
            .map_or_else(|| self.ctx.pc.clone(), |f| self.ctx.pc.join(f));
        // ADR 0117 IF2: inside `e within d`, the timing floor raises it too.
        let timing = self.ctx.timing.clone();
        let pc = timing
            .as_ref()
            .map_or_else(|| floored.clone(), |(t, _, _)| floored.join(t));
        if let Some((t, within, construct)) = timing
            && floored.flows_to(accepts)
            && !pc.flows_to(accepts)
        {
            let place = match construct {
                super::within::Construct::Within => {
                    "inside a `within` whose deadline can cut the code before it"
                }
                super::within::Construct::FirstOf => {
                    "in a branch of `first of`, which a sibling that wins can cut before it"
                }
            };
            let words = construct.words();
            let residue = self.residue_clause(accepts);
            let mut diagnostic = self.report.make(
                Code::U4003,
                span,
                format!(
                    "{what} accepts `{accepts}` data, and it runs {place}, depending on `{t}` timing, which whether it runs would reveal{residue}"
                ),
            );
            diagnostic
                .related
                .push(self.report.related(within, format!("the `{words}`")));
            self.report.push(diagnostic);
            return;
        }
        if !pc.flows_to(accepts) {
            let message = match floor {
                Some(floor) if self.ctx.pc.flows_to(accepts) => format!(
                    "{what} accepts `{accepts}` data, and it runs in an operation body whose callers may pass `{floor}` data, which it would reveal"
                ),
                _ => format!(
                    "{what} accepts `{accepts}` data, and it runs under {} `{pc}` condition, which it would reveal",
                    article(&pc.to_string())
                ),
            };
            let mut diagnostic = self.report.make(Code::U4003, span, message);
            if let Some(from) = self.ctx.pc_from {
                diagnostic
                    .related
                    .push(self.report.related(from, "the labeled condition"));
            }
            let at = self.ctx.pc.clone();
            self.name_answer(&mut diagnostic, Carrier::Condition, &at, accepts);
            self.report.push(diagnostic);
            return;
        }
        for (arg_span, ty) in args {
            let label = self.labeled(ty).sent_label(true, &self.env.kind_labels);
            if !below(&label, accepts, &self.top()) {
                let residue = self.residue_clause_for(&label, accepts);
                let mut diagnostic = self.report.make(
                    Code::U4003,
                    *arg_span,
                    format!(
                        "{what} accepts `{accepts}` data, and this argument is `{label}`{residue}"
                    ),
                );
                // ADR 0095: sending only accepted data keeps the label; a policy, which
                // widens the trusted base, comes last.
                let top = self.top();
                // An interpolated value with a labeled field: name the field (ADR 0068 D8).
                if let Some((_, field, kind, field_label)) =
                    self.written_fields
                        .iter()
                        .find(|(span, _, _, field_label)| {
                            span == arg_span && !below(field_label, accepts, &top)
                        })
                {
                    diagnostic.repairs.push(super::Report::advice(format!(
                        "field `{field}` of `{kind}` is `{field_label}`: interpolate only the fields that {what} accepts"
                    )));
                    diagnostic.repairs.push(super::Report::escape(
                        Escape::Trusted,
                        format!("declassify `{field}` through a policy of a trusted module"),
                    ));
                } else {
                    diagnostic.repairs.push(super::Report::advice(format!(
                        "send {what} only data labeled `{accepts}` or lower, or send this value to an operation that accepts `{label}`"
                    )));
                    diagnostic.repairs.push(super::Report::escape(
                        Escape::Trusted,
                        "declassify the value through a policy of a trusted module",
                    ));
                }
                self.name_answer(
                    &mut diagnostic,
                    Carrier::Value("this argument"),
                    &label,
                    accepts,
                );
                self.report.push(diagnostic);
            }
        }
    }
}

/// The indefinite article before a label written in a message: `an internal`, `a secret`
/// (R-034).
fn article(label: &str) -> &'static str {
    match label.chars().next() {
        Some('a' | 'e' | 'i' | 'o' | 'u' | 'A' | 'E' | 'I' | 'O' | 'U') => "an",
        _ => "a",
    }
}

/// The meet of the `accepts` labels of the operations in `uses`, and `None` when none of them
/// bounds its arguments: `bound_eff` of `spec/07 §7` rule 4.
pub(crate) fn bound_of(env: &super::env::Env<'_>, uses: &[FnUse]) -> Option<Label> {
    let mut bound: Option<Label> = None;
    for need in uses {
        let Some(capability) = need.capability.and_then(|c| env.capabilities.get(c)) else {
            continue;
        };
        for op in capability
            .ops
            .iter()
            .filter(|o| need.ops.iter().any(|n| n == o.name))
        {
            if let Some(accepts) = &op.accepts {
                bound = Some(match bound {
                    Some(bound) => bound.meet(accepts),
                    None => accepts.clone(),
                });
            }
        }
    }
    bound
}

/// Closes the `quiet` marks over the call graph, once every body is checked (S-39, `spec/26 §6`
/// rule 4): a call under a raised program counter is `quiet` when its callee can run a
/// construct that appends events of its own to the trace, and an operation there when the
/// package declares a handler.
///
/// A function can run one when its body, its lambdas included, holds a `for ever`, a `within`,
/// a `first of`, a `tasks`, `together` or `together for` statement or a `start`, or says
/// `diverges`; when it calls a function value, applies a function argument whose code is not
/// known at the call, installs a handler, or performs an operation while the package declares
/// a handler; or when it calls or names a function that can. A prelude function that applies a
/// function argument can too. The standard library's own bodies hold no such construct. The
/// closure is a least fixed point over the reversed call edges, linear in their number.
pub(crate) fn close_quiet(env: &super::env::Env<'_>, findings: &mut super::Findings) {
    use std::collections::{BTreeMap, BTreeSet};
    if findings.quiet_calls.is_empty() && findings.quiet_ops.is_empty() {
        return;
    }
    let handlers = env.handlers.iter().any(|h| env.in_package(h.module));
    let mut emitting: BTreeSet<usize> = env
        .functions
        .iter()
        .enumerate()
        .filter(|(_, def)| def.diverges)
        .map(|(id, _)| id)
        .collect();
    emitting.extend(findings.withins.iter().filter_map(|site| site.function));
    emitting.extend(
        findings
            .deadline_loops
            .iter()
            .filter_map(|site| site.function),
    );
    emitting.extend(findings.tasks_sites.iter().filter_map(|site| site.function));
    emitting.extend(findings.task_functions.iter().copied());
    emitting.extend(findings.value_callers.iter().copied());
    emitting.extend(findings.opaque_applies.iter().map(|&(f, _)| f));
    emitting.extend(findings.installs.iter().map(|&(f, _)| f));
    if handlers {
        emitting.extend(
            findings
                .caller_ops
                .iter()
                .filter_map(|(caller, _, _)| match caller {
                    super::timing::Caller::Function(f) => Some(*f),
                    _ => None,
                }),
        );
    }
    let mut callers: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(caller, callee, _) in &findings.edges {
        callers.entry(callee).or_default().push(caller);
    }
    let mut pending: Vec<usize> = emitting.iter().copied().collect();
    while let Some(f) = pending.pop() {
        for &caller in callers.get(&f).into_iter().flatten() {
            if emitting.insert(caller) {
                pending.push(caller);
            }
        }
    }
    let calls = std::mem::take(&mut findings.quiet_calls);
    for (span, callee) in calls {
        let applies = env
            .functions
            .get(callee)
            .is_some_and(|def| super::timing::applies_function(env, def));
        if emitting.contains(&callee) || applies {
            findings.quiet.insert(span);
        }
    }
    let ops = std::mem::take(&mut findings.quiet_ops);
    if handlers {
        findings.quiet.extend(ops);
    }
}
