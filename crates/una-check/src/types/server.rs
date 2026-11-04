//! The static rules of `HttpServer` (`spec/31 §1`, ADR 0123 rules 4 and 5, IF3, IF6, and the
//! label `L_srv` of the S4 amendment, IF-S1):
//! - **U3033** (rule 5): the effect set of a branch of `together`, of the body of a `together
//!   for` and of a branch of `first of` holds no operation of `HttpServer`: performed in the
//!   branch, needed by a function, a function value or a handler that the branch calls, names or
//!   installs, or reachable through an operation whose capability has a handler that can perform
//!   one. A branch acts for the task that runs the construct, so two of them could answer one
//!   reply right in an order that timing decides.
//! - **IF6**: a `start e` whose captures can hold a reply right, by their types, or, in a
//!   package that can hold reply rights, a function value, moves them to its task, whose end
//!   answers them through the host when the task does not. That answer is checked as a `reply`
//!   at the `start`: under `pc ⊔ τ(e)` with `within`, under `pc` without, against `internal`
//!   (U4003).
//! - **`L_srv`**: the operations of `HttpServer` and the `start`s of IF6 are the hand-off sites
//!   of the serve threads, whose work after them can shift every timing site of the package
//!   ([`super::timing::server_label`]).
//!
//! Each check is O(1) per site checked, beside a fixpoint over the handlers of the package that
//! is computed once, O(handlers × uses).

use std::collections::BTreeSet;

use una_diag::Code;
use una_syntax::Span;

use super::Findings;
use super::Report;
use super::check::Checker;
use super::env::Env;
use super::label::Label;
use super::ty::{CapId, FnUse, Ty};

/// The name of the host capability of the server (ADR 0123 rule 1).
pub(crate) const HTTP_SERVER: &str = "HttpServer";

/// A `start` whose captures can hold a reply right (IF6), checked once the package is.
#[derive(Clone, Debug)]
pub(crate) struct StartSite {
    /// The word `start`.
    pub(crate) keyword: Span,
    /// The function whose body holds it.
    pub(crate) function: Option<usize>,
    /// Whether it lies inside a lambda, a task or outside a function, for `τ_conc`.
    pub(crate) unplaced: bool,
    /// The program counter at the `start`, joined with the effect floor and the timing floor
    /// around it: what the auto-answer of its task would reveal.
    pub(crate) pc: Label,
    /// The span of the deadline of its `within`, which keys `τ(e)`, when it has one.
    pub(crate) within: Option<Span>,
    /// Whether a capture's type can hold a `ReplyRight`.
    pub(crate) reply: bool,
    /// Whether a capture's type can hold a function value.
    pub(crate) function_value: bool,
}

/// One open `start`, whose captures [`Checker::start_capture_read`] marks.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CaptureFrame {
    /// Where the locals of the started expression begin: a local before it is a capture.
    mark: usize,
    reply: bool,
    function_value: bool,
}

/// Whether `id` is the host capability `HttpServer` of `std.host` (ADR 0123 rule 1), not a
/// capability of the package that takes its name.
pub(crate) fn is_server(env: &Env<'_>, id: Option<CapId>) -> bool {
    id.and_then(|id| env.capabilities.get(id))
        .is_some_and(|c| c.module == env.prelude && c.name == HTTP_SERVER)
}

/// The capabilities whose operations can run an operation of `HttpServer`: those with a handler
/// whose `uses` hold `HttpServer`, or another such capability, to a fixpoint (U3033). The
/// handlers' `uses` are declared, so the fixpoint is over the handlers once each round, and a
/// round adds a capability or ends: O(capabilities × handlers × uses).
pub(crate) fn serving_capabilities(env: &Env<'_>) -> BTreeSet<CapId> {
    let mut serving: BTreeSet<CapId> = BTreeSet::new();
    loop {
        let before = serving.len();
        for handler in &env.handlers {
            let Some(capability) = handler.capability else {
                continue;
            };
            if serving.contains(&capability) {
                continue;
            }
            let reaches = handler.uses.iter().any(|need| {
                let used = need.usage.capability;
                is_server(env, used) || used.is_some_and(|id| serving.contains(&id))
            });
            if reaches {
                serving.insert(capability);
            }
        }
        if serving.len() == before {
            return serving;
        }
    }
}

/// Whether a value of `ty` may hold a `ReplyRight`: the type itself, or one inside a list, a
/// map, a collection, an `Option`, a `Result` or a kind's fields, at any depth (IF6). A type
/// parameter and a unification variable count as holding one, failing closed.
pub(crate) fn holds_reply(env: &Env<'_>, ty: &Ty, resolve: &dyn Fn(&Ty) -> Ty) -> bool {
    match resolve(ty) {
        Ty::ReplyRight | Ty::Param(_) | Ty::Var(_) => true,
        Ty::List(inner)
        | Ty::Dense(_, inner)
        | Ty::Labeled(_, inner)
        | Ty::Refined(_, inner)
        | Ty::Measured(_, inner) => holds_reply(env, &inner, resolve),
        Ty::Map(key, value) => holds_reply(env, &key, resolve) || holds_reply(env, &value, resolve),
        Ty::Coll(_, args) => args.iter().any(|arg| holds_reply(env, arg, resolve)),
        Ty::Kind(id, args) => {
            env.holds_reply_kinds.get(id).copied().unwrap_or(true)
                || args.iter().any(|arg| holds_reply(env, arg, resolve))
        }
        _ => false,
    }
}

/// For each kind, whether a value of it may hold a `ReplyRight` through its fields, at any
/// depth: one pass over the fields, then the users of each kind found, O(kinds + fields).
pub(crate) fn reply_kinds(env: &Env<'_>) -> Vec<bool> {
    super::crossing::kinds_holding(env, &|ty: &Ty| matches!(ty, Ty::ReplyRight))
}

impl Checker<'_, '_, '_> {
    /// U3033 for the operation `@cap.op` at `span` of the capability `capability`, inside a
    /// branch of `together`, `together for` or `first of`: an operation of `HttpServer`, or of
    /// a capability whose handlers can perform one (ADR 0123 rule 5).
    pub(crate) fn branch_server_op(
        &mut self,
        capability: Option<CapId>,
        cap: &str,
        op: &str,
        span: Span,
    ) {
        let Some((words, construct)) = self.server_branch else {
            return;
        };
        if self.pass != super::authority::Pass::Check {
            return;
        }
        let message = if is_server(self.env, capability) {
            format!("`@{cap}.{op}` cannot run in {words}; answer after the block")
        } else if capability.is_some_and(|id| self.env.serving_capabilities.contains(&id)) {
            format!(
                "`@{cap}.{op}` can run a handler that performs an operation of `HttpServer`, which cannot run in {words}; answer after the block"
            )
        } else {
            return;
        };
        self.branch_server_error(span, message, construct);
    }

    /// U3033 for `callee` at `span`, whose requirements `needs` hold `HttpServer` or a
    /// capability whose handlers can perform one, inside a branch (ADR 0123 rule 5): a call, a
    /// call of a function value, a function named or passed as a value, or a handler
    /// installed.
    pub(crate) fn branch_server_needs(&mut self, callee: &str, needs: &[FnUse], span: Span) {
        let Some((words, construct)) = self.server_branch else {
            return;
        };
        if self.pass != super::authority::Pass::Check {
            return;
        }
        let serving = needs.iter().any(|need| {
            is_server(self.env, need.capability)
                || need
                    .capability
                    .is_some_and(|id| self.env.serving_capabilities.contains(&id))
        });
        if !serving {
            return;
        }
        let message = format!(
            "{callee} can perform an operation of `HttpServer`, which cannot run in {words}; answer after the block"
        );
        self.branch_server_error(span, message, construct);
    }

    fn branch_server_error(&mut self, span: Span, message: String, construct: Span) {
        let mut diagnostic = self.report.make(Code::U3033, span, message);
        diagnostic.related.push(
            self.report
                .related(construct, "the construct whose branches run at once"),
        );
        diagnostic.repairs.push(Report::advice(
            "move the operation after the construct, and compute in its branches",
        ));
        self.report.push(diagnostic);
    }

    /// Records an operation of `HttpServer` at `span`, a hand-off site of `L_srv` (IF-S1).
    pub(crate) fn server_site(&mut self, capability: Option<CapId>, span: Span) {
        if self.pass != super::authority::Pass::Check || !is_server(self.env, capability) {
            return;
        }
        let site = super::timing::HttpSite {
            span,
            function: self.current,
            unplaced: self.unplaced(),
        };
        self.findings.server_sites.push(site);
    }

    /// Opens the captures of a `start` whose expression is about to be checked (IF6).
    pub(crate) fn start_capture_open(&mut self) {
        self.start_captures.push(CaptureFrame {
            mark: self.locals.len(),
            reply: false,
            function_value: false,
        });
    }

    /// Marks, in every open `start` that captures it, a read of the local at `position` of
    /// type `ty`: whether it can hold a reply right or a function value.
    pub(crate) fn start_capture_read(&mut self, position: usize, ty: &Ty) {
        if self.start_captures.is_empty() {
            return;
        }
        let reply = holds_reply(self.env, ty, &|ty: &Ty| self.resolve(ty));
        let function_value = self.holds_function(ty);
        for frame in &mut self.start_captures {
            if position < frame.mark {
                frame.reply |= reply;
                frame.function_value |= function_value;
            }
        }
    }

    /// Closes the captures of the `start` at `keyword`, once its expression is checked, and
    /// records it when a capture can hold a reply right or a function value (IF6).
    pub(crate) fn start_capture_close(&mut self, keyword: Span, within: Option<Span>) {
        let Some(frame) = self.start_captures.pop() else {
            return;
        };
        if self.pass != super::authority::Pass::Check
            || !(frame.reply || frame.function_value)
            || !self.labels()
        {
            return;
        }
        let mut pc = self.ctx.pc.clone();
        if let Some(floor) = &self.ctx.floor {
            pc = pc.join(floor);
        }
        if let Some((timing, _, _)) = &self.ctx.timing {
            pc = pc.join(timing);
        }
        self.findings.server_starts.push(StartSite {
            keyword,
            function: self.current,
            unplaced: self.unplaced(),
            pc,
            within,
            reply: frame.reply,
            function_value: frame.function_value,
        });
    }
}

/// Whether the `start` of `site` moves reply rights in this package: a capture that can hold
/// one by its type, or a function value in a package that can hold reply rights (IF6).
pub(crate) fn moves_rights(env: &Env<'_>, site: &StartSite) -> bool {
    site.reply || (site.function_value && env.reply_rights)
}

/// IF6 after pass 2 (ADR 0123, `spec/31 §1`): each `start` that moves reply rights is checked as
/// a `reply` of the auto-answer its task's end may send, under its program counter, joined with
/// `τ(e)` when it has `within`, against `internal`: U4003 at the `start` otherwise. O(sites).
pub(crate) fn start_rights(env: &Env<'_>, findings: &Findings, report: &mut Report<'_>) {
    let internal = Label {
        level: una_syntax::ast::Level::Internal,
        ..Label::public()
    };
    let mut seen: BTreeSet<Span> = BTreeSet::new();
    for site in &findings.server_starts {
        if !moves_rights(env, site) || !seen.insert(site.keyword) {
            continue;
        }
        let tau = site
            .within
            .and_then(|key| env.within_timing.get(&key).cloned())
            .unwrap_or_else(Label::public);
        let label = site.pc.join(&tau);
        if label.flows_to(&internal) {
            continue;
        }
        let what = if site.reply {
            "its captures can hold a reply right"
        } else {
            "it captures a function value, which can hold a reply right"
        };
        let why = if site.pc.flows_to(&internal) {
            format!("its deadline depends on `{tau}` timing")
        } else {
            let pc = &site.pc;
            format!("it runs under a `{pc}` condition")
        };
        let mut diagnostic = report.make(
            Code::U4003,
            site.keyword,
            format!(
                "this `start` moves the reply rights its task holds, since {what}, and the host answers each one its task leaves unanswered, which accepts `internal` data; {why}, which that answer would reveal"
            ),
        );
        diagnostic.repairs.push(Report::advice(
            "start the task under an `internal` condition at most, without a deadline whose timing is above `internal`, and reply inside it",
        ));
        report.push(diagnostic);
    }
}
