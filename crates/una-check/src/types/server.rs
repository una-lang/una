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
//!   at the `start`: under `pc`, joined with `τ(e)` with `within`, against `internal` (U4003).
//!   And a task that can recurse and can hold data above `internal` is refused, since its
//!   depth decides whether it ends alone past `--task-depth` and is answered `500` (#1674,
//!   [`start_rights`]).
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
    /// The seed of `e`, for what its task reaches (#1674).
    pub(crate) seed: super::timing::TimingSeed,
}

/// One open `start`, whose captures [`Checker::start_capture_read`] marks.
#[derive(Clone, Debug)]
pub(crate) struct CaptureFrame {
    /// Where the locals of the started expression begin: a local before it is a capture.
    mark: usize,
    /// Whether a capture's type can hold a `ReplyRight`.
    reply: bool,
    /// Whether a capture's type can hold a function value.
    function_value: bool,
    /// The source names of the captures that can hold a reply right, which the engines walk
    /// at the start for the rights that move (ADR 0123 rule 4.2).
    names: BTreeSet<String>,
}

/// Whether `id` is the host capability `HttpServer` of `std.host` (ADR 0123 rule 1), not a
/// capability of the package that takes its name.
pub(crate) fn is_server(env: &Env<'_>, id: Option<CapId>) -> bool {
    id.and_then(|id| env.capabilities.get(id))
        .is_some_and(|c| c.module == env.prelude && c.name == HTTP_SERVER)
}

/// The capabilities whose operations can run an operation of `HttpServer`: those with a handler
/// whose `uses` hold `HttpServer`, or another such capability, to a fixpoint (U3033). A search
/// over the edges from each capability to the capabilities of the handlers that use it, from
/// the handlers that use `HttpServer`: O(capabilities + handlers × uses).
pub(crate) fn serving_capabilities(env: &Env<'_>) -> BTreeSet<CapId> {
    // `users[c]`: the capabilities of the handlers whose `uses` hold `c`.
    let mut users: std::collections::BTreeMap<CapId, Vec<CapId>> =
        std::collections::BTreeMap::new();
    let mut serving: BTreeSet<CapId> = BTreeSet::new();
    let mut pending: Vec<CapId> = Vec::new();
    for handler in &env.handlers {
        let Some(capability) = handler.capability else {
            continue;
        };
        for need in &handler.uses {
            let used = need.usage.capability;
            if is_server(env, used) {
                if serving.insert(capability) {
                    pending.push(capability);
                }
            } else if let Some(used) = used {
                users.entry(used).or_default().push(capability);
            }
        }
    }
    while let Some(capability) = pending.pop() {
        for &user in users.get(&capability).map_or(&[][..], Vec::as_slice) {
            if serving.insert(user) {
                pending.push(user);
            }
        }
    }
    serving
}

/// Whether a value of `ty` may hold a `ReplyRight`: the type itself, or one inside a list, a
/// map, a collection, an `Option`, a `Result` or a kind's fields, at any depth (IF6). A type
/// parameter and a unification variable count as holding one, failing closed.
pub(crate) fn holds_reply(env: &Env<'_>, ty: &Ty, resolve: &dyn Fn(&Ty) -> Ty) -> bool {
    match resolve(ty) {
        Ty::ReplyRight => true,
        // A value of a type parameter or an unsolved variable holds a reply right only in a
        // package where a value can hold one at all: one that imports `HttpServer`, `Exchange`
        // or `ReplyRight`, the checker seeing every module of the program (ADR 0123 IF6).
        Ty::Param(_) | Ty::Var(_) => env.reply_rights,
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

    /// U3033 for the `uses of` parameter `name`, called or passed on at `span` inside a branch,
    /// in a program that imports a name of the server (ADR 0123 rule 5): its argument's effects
    /// are its callers', unknown here, and a caller outside the branch may pass a lambda that
    /// replies, so the branch could answer a reply right beside another. Failing closed, as
    /// the most secure reading of the effect set of the branch (`spec/05 §3` rule 3).
    pub(crate) fn branch_server_of_param(&mut self, name: &str, span: Span) {
        let Some((words, construct)) = self.server_branch else {
            return;
        };
        if self.pass != super::authority::Pass::Check || !self.env.server_imported {
            return;
        }
        let message = format!(
            "`{name}` runs the code its callers pass, which in a program that serves can perform an operation of `HttpServer`, and that cannot run in {words}; call it after the block"
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
        // The seed of `e`, for the depth of its task (#1674).
        self.timing_start();
        self.start_captures.push(CaptureFrame {
            mark: self.locals.len(),
            reply: false,
            function_value: false,
            names: BTreeSet::new(),
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
        // A capture the engines walk for reply rights: one whose type can hold a right, or a
        // function value in a package where a value can hold one (IF6, rule 4.2).
        let walked = reply || (function_value && self.env.reply_rights);
        let name = self.locals.get(position).map(|local| local.name.to_owned());
        for frame in &mut self.start_captures {
            if position < frame.mark {
                frame.reply |= reply;
                frame.function_value |= function_value;
                if walked && let Some(name) = &name {
                    frame.names.insert(name.clone());
                }
            }
        }
    }

    /// Closes the captures of the `start` at `keyword`, once its expression is checked, and
    /// records it when a capture can hold a reply right or a function value (IF6).
    pub(crate) fn start_capture_close(&mut self, keyword: Span, within: Option<Span>) {
        let seed = self.timing_end();
        let Some(frame) = self.start_captures.pop() else {
            return;
        };
        // What the engines walk at this `start`, whatever the labels (rule 4.2).
        if self.pass == super::authority::Pass::Check && !frame.names.is_empty() {
            self.findings
                .start_rights
                .entry(keyword)
                .or_default()
                .extend(frame.names.iter().cloned());
        }
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
            seed,
        });
    }
}

/// Whether the `start` of `site` moves reply rights in this package: a capture that can hold
/// one by its type, or a function value in a package that can hold reply rights (IF6).
pub(crate) fn moves_rights(env: &Env<'_>, site: &StartSite) -> bool {
    site.reply || (site.function_value && env.reply_rights)
}

/// IF6 after pass 2 (ADR 0123, `spec/31 §1`): each `start` that moves reply rights is checked as
/// a `reply` of the auto-answer its task's end may send, under its program counter, and under
/// `τ(e)` when it has `within`, against `internal`: U4003 at the `start` otherwise.
///
/// **The depth of its task** (#1674). A started task that runs past `--task-depth` ends alone
/// and its rights are answered `500`, so the status a peer reads may tell how deep the task
/// went. The `start` is refused too when both:
/// - its task can push frames without a bound its code shows: it reaches a cycle of the call
///   graph (a function, a handler, a policy's `via`, or `Λ`), a call of a function value or of
///   a `uses of` parameter, or code the findings do not hold (a standard module other than the
///   prelude, a dependency not checked with the package);
/// - it can hold data above `internal`: its program counter, what it reads from outside, what
///   it binds or receives, or what any node it reaches declares, binds or receives.
///
/// That is coarse, and sound: without the first, its depth is bounded by its code alone, the
/// same in every run; without the second, the depth depends on `internal` data at most, which
/// the answer accepts. The prelude's functions are host code in both engines and push no frame
/// of Una code but through a function argument, which is a call of a function value.
///
/// O(V + E) once for the package ([`super::timing::start_reaches`]), then O(size of the seed) per
/// `start`.
pub(crate) fn start_rights(env: &Env<'_>, findings: &Findings, report: &mut Report<'_>) {
    let internal = Label {
        level: una_syntax::ast::Level::Internal,
        ..Label::public()
    };
    let mut seen: BTreeSet<Span> = BTreeSet::new();
    // Built once, at the first `start` that moves reply rights.
    let mut reach = None;
    for site in &findings.server_starts {
        if !moves_rights(env, site) || !seen.insert(site.keyword) {
            continue;
        }
        let tau = site
            .within
            .and_then(|key| env.within_timing.get(&key).cloned())
            .unwrap_or_else(Label::public);
        let reach = reach.get_or_insert_with(|| super::timing::start_reaches(env, findings));
        let found = super::timing::start_reach(reach, &site.seed, site.function_value, &internal);
        let deep = found.unbounded.is_some() && !found.label.flows_to(&internal);
        if site.pc.join(&tau).flows_to(&internal) && !deep {
            continue;
        }
        let what = if site.reply {
            "its captures can hold a reply right"
        } else {
            "it captures a function value, which can hold a reply right"
        };
        let why = if !site.pc.flows_to(&internal) {
            let pc = &site.pc;
            format!("it runs under a `{pc}` condition")
        } else if !tau.flows_to(&internal) {
            format!("its deadline depends on `{tau}` timing")
        } else {
            let how = match found.unbounded {
                Some(super::timing::Unbounded::Cycle(node)) => {
                    format!("its task can recurse, through {}", reach.name(env, node))
                }
                Some(super::timing::Unbounded::Unseen(node)) => format!(
                    "its task calls {}, whose code the checker does not see",
                    reach.name(env, node)
                ),
                _ => {
                    "its task calls a function value, whose code the call does not show".to_owned()
                }
            };
            let data = match (&found.source, found.labeled) {
                (Some(source), _) => source.clone(),
                (None, Some(node)) => format!("the data of {}", reach.name(env, node)),
                (None, None) => "its data".to_owned(),
            };
            format!(
                "{how}, and it holds `{}` data, {data}, which may decide how deep it goes; a started task that runs past `--task-depth` ends alone and its rights are answered `500`",
                found.label
            )
        };
        let mut diagnostic = report.make(
            Code::U4003,
            site.keyword,
            format!(
                "this `start` may move reply rights to its task, since {what}, and the host answers each one its task leaves unanswered, which accepts `internal` data; {why}, which that answer would reveal"
            ),
        );
        diagnostic.repairs.push(Report::advice(
            "start the task under an `internal` condition at most, without a deadline whose timing is above `internal`, and reply inside it; a task that recurses or calls a function value holds data at most `internal`, in what it captures, reads and calls",
        ));
        report.push(diagnostic);
    }
}
