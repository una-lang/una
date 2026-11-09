//! The operations of `HttpServer` in the interpreter (ADR 0123 rules 1 to 4, 14 and 18,
//! `spec/31 §1`), over the serve engine of `crates/una-http` that native code shares.
//!
//! - **`next`** checks `close` and the host stop first, so a closed or stopped run never
//!   delivers; then it asks the engine for a request, which is one atomic step (the request is
//!   taken off its queue and becomes an exchange, or it stays queued). With none to deliver it
//!   lets the other ready tasks run, and only when none is ready does it wait for the engine,
//!   bounded by the deadlines in force and ended by a host stop. A wait takes no request, so a
//!   `next` that fails leaves every request queued (rule 4.7).
//! - **Reply rights** live in the run's [`Rights`]: `next` gives the right to the task that
//!   called it, `start` moves the rights its captures reach ([`Machine::move_rights`]), `reply`
//!   checks the holder (`NotHeld`), and a task's end answers what it still holds
//!   ([`Machine::answer_held`]).
//! - **`reply`** checks and hands the reply to the engine, which never waits on a peer; a reply
//!   that fails its checks (`BadReply`) sends nothing and keeps the right; one that another
//!   transition of rule 4.7 overtook returns `unit` and sends nothing.
//!
//! Each operation is O(1) beside the bytes of the request or the reply; moving rights at a
//! `start` costs one look at each value reachable from the captures that can hold a right.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use una_core::{Ctor, Name, Path};
use una_http::serve::program::{Port, Waited, peer_text};
use una_http::serve::rights::{Rights, Task, Word};
use una_http::serve::run::{Exchange, Next, Sent};
use una_http::serve::trace::TraceKey;
use una_syntax::Span;

use super::{Abort, CANCEL, Event, EventOutcome, Flow, Machine, Stop, fault, prelude_ctor};
use crate::env::Env;
use crate::value::{Elements, Value};

/// The status of the auto-answer to a right its task still holds when it returned, failed or
/// ended alone (ADR 0123 rule 4.5).
pub(crate) const UNANSWERED: u16 = 500;

/// The status of the auto-answer to a right its task still holds when it was cancelled, cut by
/// `within` or stopped, with `Retry-After: 1` (ADR 0123 rule 4.5).
pub(crate) const STOPPED: u16 = 503;

/// The serve engine of a run, as its program meets it, and the run's reply rights.
pub(crate) struct Serving<'w> {
    port: &'w dyn Port,
    rights: Rights<Box<Exchange>>,
    /// Set by `close`, once: every later `next` fails with `Closed`.
    closed: Cell<bool>,
    /// The key of the run's reduced trace form, and whether the trace writes the server's
    /// values without their labels' redaction ([`Port::reveals_trace`], the tests only).
    trace: (TraceKey, bool),
}

impl<'w> Serving<'w> {
    /// The program's side of the engine `port`, with no right yet.
    pub(crate) fn new(port: &'w dyn Port) -> Serving<'w> {
        Serving {
            port,
            rights: Rights::new(),
            closed: Cell::new(false),
            trace: (port.trace_key(), port.reveals_trace()),
        }
    }

    /// The key of the run's reduced trace form (ADR 0123 rule 1), and whether the trace writes
    /// the server's values without their labels' redaction (the toolchain's tests only).
    pub(crate) fn trace(&self) -> (TraceKey, bool) {
        self.trace
    }

    /// Answers `503` each right `task` still holds, in the order it received them, and appends
    /// no event: the task whose body never ran, or ended by a panic of the interpreter, as a
    /// stopped task (rule 4.5). O(rights held).
    pub(crate) fn answer_left(&self, task: Task) {
        for (_, mut exchange) in self.rights.end(task) {
            exchange.answer(STOPPED);
        }
    }

    /// The rights live in the run, none once every task answered its own (for tests).
    pub(crate) fn live(&self) -> usize {
        self.rights.live()
    }
}

/// A task waiting in `next`, recorded in the run's waiters until it leaves `next`.
struct Waiting<'r, 'p, 'w> {
    run: &'r crate::tasks::Run<'p, 'w>,
    task: usize,
}

impl<'r, 'p, 'w> Waiting<'r, 'p, 'w> {
    fn enter(
        run: &'r crate::tasks::Run<'p, 'w>,
        task: usize,
        cancels: Vec<Rc<crate::tasks::TasksScope>>,
    ) -> Self {
        let waiter = crate::tasks::NextWaiter {
            cancels,
            until: Cell::new(None),
        };
        let fresh = run.next_waiters.borrow_mut().insert(task, waiter).is_none();
        // INVARIANT: a task runs one operation at a time, so it enters `next` once.
        debug_assert!(fresh, "task {task} entered `next` twice");
        Waiting { run, task }
    }

    /// Records the deadline that ends this wait, on the run's clock.
    fn until(&self, until: Option<u64>) {
        if let Some(waiter) = self.run.next_waiters.borrow().get(&self.task) {
            waiter.until.set(until);
        }
    }

    /// The earliest deadline of the other waiters, which the wait of this one may not outlast,
    /// so that a waiter whose deadline passed runs to end its `next`.
    fn others_until(&self) -> Option<u64> {
        self.run
            .next_waiters
            .borrow()
            .iter()
            .filter(|(id, _)| **id != self.task)
            .filter_map(|(_, waiter)| waiter.until.get())
            .min()
    }
}

impl Drop for Waiting<'_, '_, '_> {
    fn drop(&mut self) {
        let left = self
            .run
            .next_waiters
            .borrow_mut()
            .remove(&self.task)
            .is_some();
        // INVARIANT: only this guard removes its task, once.
        debug_assert!(left, "task {} left `next` twice", self.task);
    }
}

/// Answers `503`, with no event, the rights the root task still holds when it unwinds by a
/// panic of the interpreter, as [`crate::tasks::Ending`] does for a started task (ADR 0123 rule
/// 4.5); on a normal end the root answered them already.
pub(crate) struct RootRights<'r, 'p, 'w>(pub(crate) &'r crate::tasks::Run<'p, 'w>);

impl Drop for RootRights<'_, '_, '_> {
    fn drop(&mut self) {
        if std::thread::panicking()
            && let Some(serving) = &self.0.serve
        {
            serving.answer_left(0);
        }
    }
}

/// When a wait for the engine ends at the latest, on the run's clock: at the earliest of the
/// waiter's own deadline, `own`, and the other waiters' earliest, `others`, so that a waiter
/// whose deadline passes runs to end its `next`; `None` when no waiter has one.
pub(crate) fn wake_at(own: Option<u64>, others: Option<u64>) -> Option<u64> {
    match (own, others) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Whether a wait that timed out at `wake` reached the waiter's own deadline `own`, which ends its
/// `next` with `TimedOut`; otherwise another waiter's deadline woke it, and it waits again.
pub(crate) fn own_deadline(own: Option<u64>, wake: Option<u64>) -> bool {
    own.is_some() && wake == own
}

/// A failure of `ServeError` (`spec/31 §1`), by its variant's name and position.
fn serve_error<'p>(name: &str, index: usize) -> Stop<'p> {
    Stop::Fail(Value::data(
        prelude_ctor("ServeError", name, index),
        Vec::new(),
    ))
}

fn interrupted<'p>() -> Stop<'p> {
    serve_error("Interrupted", 0)
}

fn timed_out<'p>() -> Stop<'p> {
    serve_error("TimedOut", 1)
}

fn closed<'p>() -> Stop<'p> {
    serve_error("Closed", 2)
}

fn not_held<'p>() -> Stop<'p> {
    serve_error("NotHeld", 3)
}

fn bad_reply<'p>() -> Stop<'p> {
    serve_error("BadReply", 4)
}

/// The `Method` of `std.host` for the engine's method (`spec/31 §1`).
fn method<'p>(method: una_http::strict::Method) -> Value<'p> {
    use una_http::strict::Method as M;
    let (name, index) = match method {
        M::Get => ("Get", 0),
        M::Head => ("Head", 1),
        M::Post => ("Post", 2),
        M::Put => ("Put", 3),
        M::Patch => ("Patch", 4),
        M::Delete => ("Delete", 5),
        M::Options => ("Options", 6),
    };
    Value::data(prelude_ctor("Method", name, index), Vec::new())
}

fn header<'p>(name: &str, value: &str) -> Value<'p> {
    Value::data(
        prelude_ctor("Header", "Header", 0),
        vec![Value::text(name), Value::text(value)],
    )
}

/// The `Request` of `std.host` for what the engine read (`spec/31 §1`): one copy of its bytes
/// into values.
fn request<'p>(exchange: &Exchange) -> Value<'p> {
    request_value(
        &exchange.request,
        &peer_text(exchange.peer()),
        exchange.listener(),
    )
}

/// The `Request` of `std.host` for `read`, from `peer` through `listener`.
fn request_value<'p>(read: &una_http::request::Request, peer: &str, listener: &str) -> Value<'p> {
    let segments = read.segments.iter().map(|s| Value::text(s)).collect();
    let params = read
        .query
        .iter()
        .map(|(name, value)| {
            Value::data(
                prelude_ctor("Param", "Param", 0),
                vec![Value::text(name), Value::text(value)],
            )
        })
        .collect();
    let headers = read
        .headers
        .iter()
        .map(|(name, value)| header(name, value))
        .collect();
    Value::data(
        prelude_ctor("Request", "Request", 0),
        vec![
            method(read.method),
            Value::text(&read.target),
            Value::List(Rc::new(segments)),
            Value::List(Rc::new(params)),
            Value::List(Rc::new(headers)),
            Value::text(&read.body),
            Value::text(peer),
            Value::text(listener),
        ],
    )
}

/// The peer of the requests of `std.testing.test_request`, from the block of RFC 5737 for
/// documentation (ADR 0123 rule 21).
const TEST_PEER: &str = "192.0.2.1";

/// The listener of the requests of `std.testing.test_request` (ADR 0123 rule 21).
const TEST_LISTENER: &str = "test";

/// The engine's method for a `Method` of `std.host`, by its variant's position.
fn method_of(value: Option<&Value<'_>>) -> Option<una_http::strict::Method> {
    use una_http::strict::Method as M;
    let Some(Value::Data(data)) = value else {
        return None;
    };
    [
        M::Get,
        M::Head,
        M::Post,
        M::Put,
        M::Patch,
        M::Delete,
        M::Options,
    ]
    .get(data.ctor.index)
    .copied()
}

/// `std.testing.test_request(method, target, body)` (ADR 0123 rule 21): the request the host
/// would read, by the host's own reader (`una_http::request::test_request`), or `Refused` of
/// `TestRequestError` where the host would answer `400`. O(t + b) for a target of t bytes and a
/// body of b bytes.
pub(super) fn test_request<'p>(args: &[Value<'p>]) -> Flow<'p> {
    let (Some(method), Some(Value::Text(target)), Some(Value::Text(body))) =
        (method_of(args.first()), args.get(1), args.get(2))
    else {
        return Err(fault(
            "`test_request` without a method, a target and a body",
        ));
    };
    match una_http::request::test_request(method, target, body) {
        Ok(read) => Ok(request_value(&read, TEST_PEER, TEST_LISTENER)),
        Err(_) => Err(Stop::Fail(Value::data(
            Ctor {
                kind: Path {
                    module: "std.testing".to_owned(),
                    name: "TestRequestError".to_owned(),
                },
                name: "Refused".to_owned(),
                index: 0,
            },
            Vec::new(),
        ))),
    }
}

impl<'p> Machine<'p, '_> {
    /// `std.testing.test_exchange(request)` (ADR 0123 rule 21): `request` with a fresh reply
    /// right. Its word has the generation 0, which no word the run's table gives has
    /// ([`Rights`] starts every slot at 1, and a retired slot holds no entry), so it is distinct
    /// from every right `next` gives, and a `reply` of the host's on it is `NotHeld`; the slot half counts the test's rights, so
    /// each is distinct from the others. O(1).
    pub(super) fn test_exchange(&mut self, args: &[Value<'p>]) -> Flow<'p> {
        let Some(request) = args.first() else {
            return Err(fault("`test_exchange` without a request"));
        };
        let count = self.run.test_rights.get();
        let Some(next) = count.checked_add(1) else {
            return Err(fault("more reply rights than a test can make"));
        };
        self.run.test_rights.set(next);
        Ok(Value::data(
            prelude_ctor("Exchange", "Exchange", 0),
            vec![request.clone(), Value::Right(u64::from(count) << 32)],
        ))
    }
}

/// The engine's reply for a `Reply` of `std.host`: a status out of range stays out of range,
/// which the engine's checks refuse (`BadReply`).
fn reply_of(value: Option<&Value<'_>>) -> una_http::reply::Reply {
    let text = |value: Option<&Value<'_>>| match value {
        Some(Value::Text(t)) => t.to_string(),
        _ => String::new(),
    };
    let mut reply = una_http::reply::Reply {
        status: 0,
        headers: Vec::new(),
        body: String::new(),
    };
    if let Some(Value::Data(data)) = value {
        reply.status = match data.fields.first() {
            Some(Value::Int(n)) => i64::try_from(n)
                .ok()
                .and_then(|n| u16::try_from(n).ok())
                .unwrap_or(0),
            _ => 0,
        };
        if let Some(Value::List(items)) = data.fields.get(1) {
            reply.headers = items
                .iter()
                .map(|item| match item {
                    Value::Data(h) => (text(h.fields.first()), text(h.fields.get(1))),
                    _ => (String::new(), String::new()),
                })
                .collect();
        }
        reply.body = text(data.fields.get(2));
    }
    reply
}

/// Whether a task ended with `result` by the hard stop (ADR 0116 rule 16, ADR 0123 rule 18 item
/// 6): its rights are answered `503` and append no `unanswered` event, as in native code, which
/// ends the process at the hard stop without waiting for its tasks (`spec/31 §1` rule 3 item 5).
pub(crate) fn hard_stopped(result: &Flow<'_>) -> bool {
    matches!(result, Err(Stop::Abort(Abort::HostStop { .. })))
}

/// The status of the auto-answers of a task that ended with `result` (rule 4.5): `500` when it
/// returned, failed or ended alone past its depth; `503` when it was cancelled, cut by `within`
/// or stopped, the run's stop by an abort included. `alone` says whether the task is a started
/// one, whose depth abort ends it alone.
pub(crate) fn status_of(result: &Flow<'_>, alone: bool) -> u16 {
    match result {
        Ok(_) | Err(Stop::Fail(_) | Stop::Exit(..)) => UNANSWERED,
        Err(Stop::Abort(Abort::Depth { .. })) if alone => UNANSWERED,
        Err(Stop::Abort(_) | Stop::Cut(_)) => STOPPED,
    }
}

impl<'p> Machine<'p, '_> {
    /// The task that runs, as the run's reply rights know it: the root is 0.
    fn holder(&self) -> Task {
        self.run.current() as Task
    }

    /// An operation of `HttpServer` on the host's instance.
    pub(super) fn server(&mut self, op: &str, args: &[Value<'p>], span: Span) -> Flow<'p> {
        if self.run.serve.is_none() {
            // A run whose entry functions use `HttpServer` starts its engine before `main`
            // (`spec/31 §6` rule 14), and the checker requires a `serve:` grant for them
            // (U3012), so only a defect reaches here; it fails closed.
            return Err(fault(format!(
                "`HttpServer.{op}` in a run that serves nothing"
            )));
        }
        match op {
            "next" => self.server_next(span),
            "reply" => self.server_reply(args),
            "close" => {
                if let Some(serving) = &self.run.serve
                    && !serving.closed.replace(true)
                {
                    serving.port.close();
                }
                Ok(Value::Unit)
            }
            _ => Err(fault(format!("no host operation `HttpServer.{op}`"))),
        }
    }

    /// `next` (ADR 0123 rules 1, 4.1, 4.7 and 18): `Closed` after `close`, `Interrupted` once a
    /// host stop came, else the next request delivered, which the calling task holds the right
    /// of; it waits, parked, while none can be delivered, and its deadline, a `within`'s or a
    /// `for ever within`'s, ends the wait with `TimedOut`.
    fn server_next(&mut self, span: Span) -> Flow<'p> {
        let run = Rc::clone(&self.run);
        let Some(serving) = run.serve.as_ref() else {
            return Err(fault("`HttpServer.next` in a run that serves nothing"));
        };
        // The run's first server operation begins accepting (S4 amendment B.2).
        serving.port.begin();
        // While it waits here, the task is one of the run's waiters in `next` (`yield_turn`),
        // on every way out.
        let cancels = self
            .cancels
            .iter()
            .skip(self.cancel_shield)
            .cloned()
            .collect();
        let waiting = Waiting::enter(&run, run.current(), cancels);
        loop {
            // An abort of another task ends the whole run (`spec/27 §7`), and this `next` ends
            // with it, taking no request. It comes first: a `close` or a host stop that came
            // with the abort does not turn the run's end into a catchable failure of this task
            // (`spec/31 §1` rule 3 item 5).
            if let Some(abort) = run.aborted() {
                return Err(Stop::Abort(abort));
            }
            if serving.closed.get() {
                return Err(closed());
            }
            // A stopped run never delivers: the engine refuses what it has not (rule 18).
            if self.stop.stopped() {
                return Err(interrupted());
            }
            let ever = self
                .deadline
                .as_ref()
                .and_then(|deadline| deadline.left(&self.run.clock));
            let within = self.within_left();
            let (left, by_ever) = match (ever, within) {
                (Some(ever), Some(within)) if within < ever => (Some(within), false),
                (Some(ever), _) => (Some(ever), true),
                (None, within) => (within, false),
            };
            let epoch = serving.port.epoch();
            let retry = match serving.port.next() {
                Next::Exchange(exchange) => return self.deliver(serving, exchange),
                // The engine stopped the run by a fault (`spec/31 §6` rule 13): U8001.
                Next::Failed(failure) => {
                    return Err(Stop::Abort(Abort::Fault {
                        message: failure.to_string(),
                        span: Some(span),
                    }));
                }
                Next::Blocked(retry) => retry,
                Next::Empty => None,
            };
            // Past its deadline, the wait ends with no request taken. Under virtual time a wait
            // for the network takes no virtual time (ADR 0116 rule 19), so only a deadline
            // that has passed cuts it.
            if left.is_some_and(|left| left.is_zero()) {
                if by_ever {
                    self.timeout_event();
                }
                return Err(timed_out());
            }
            let now = self.run.clock.now();
            let until = left.map(|left| {
                now.saturating_add(u64::try_from(left.as_millis()).unwrap_or(u64::MAX))
            });
            waiting.until(until);
            // Another ready task runs first: it may answer an exchange, which frees a `/once`
            // slot, or end, which another task waits for, or it is a waiter that its
            // cancellation or its deadline ended.
            if run.yield_turn() {
                // An abort of the run that came meanwhile comes first, as at the loop's head; a
                // cancellation that came with it does not make the operation `interrupted`.
                if let Some(abort) = run.aborted() {
                    return Err(Stop::Abort(abort));
                }
                if self.cancelled() {
                    return Err(Stop::Cut(CANCEL));
                }
                continue;
            }
            // The wait lasts no longer than the earliest deadline of any waiter, so one whose
            // deadline passes runs; only its own ends this `next` with `TimedOut`.
            let wake = wake_at(until, waiting.others_until());
            let deadline = match wake {
                Some(at) if !self.run.clock.is_virtual() => std::time::Instant::now()
                    .checked_add(std::time::Duration::from_millis(at.saturating_sub(now))),
                _ => None,
            };
            let stop = std::sync::Arc::clone(&self.stop);
            // No other task runs while this one blocks, so an abort cannot come during the
            // wait today; the term keeps the wait correct should one ever come from elsewhere.
            let aborted = || run.aborted().is_some();
            // A task's `Http` request that landed off the thread ends the wait too, within the
            // engine's look of 10 ms, so that the task runs on (#1730); the next pass of the loop
            // yields to it.
            match serving.port.wait(epoch, retry, deadline, &|| {
                stop.stopped() || aborted() || run.landed()
            }) {
                // The loop's head says which: a host stop, an abort of the run, or a landing.
                Waited::Changed | Waited::Stopped => {}
                Waited::TimedOut if !own_deadline(until, wake) => {
                    // Another waiter's deadline passed: it runs, and this one waits again.
                }
                Waited::TimedOut => {
                    // The call returns once the run's clock has reached the deadline, as an
                    // `Http` request does (`spec/27 §5` rule 6, #1355).
                    if let Some(at) = until {
                        self.run.clock.wait_until(at);
                    }
                    if by_ever {
                        self.timeout_event();
                    }
                    return Err(timed_out());
                }
            }
        }
    }

    /// The exchange `next` took, as an `Exchange` of `std.host`, its right held by the task
    /// that runs (rule 4.1): the request's values are built first, then the right is recorded,
    /// so nothing can fail between the record and the return.
    fn deliver(&mut self, serving: &Serving<'_>, exchange: Box<Exchange>) -> Flow<'p> {
        let request = request(&exchange);
        match serving.rights.deliver(self.holder(), exchange) {
            Ok(right) => Ok(Value::data(
                prelude_ctor("Exchange", "Exchange", 0),
                vec![request, Value::Right(right)],
            )),
            Err(mut exchange) => {
                // The table cannot take it: the host answers it, and the run stops, failing
                // closed.
                exchange.answer(STOPPED);
                Err(Stop::Abort(Abort::Fault {
                    message: "the run's table of reply rights cannot take another exchange"
                        .to_owned(),
                    span: None,
                }))
            }
        }
    }

    /// `reply(to, reply)` (rules 4.4, 4.7, 13 and 14): `NotHeld` for a right the task does not
    /// hold, `BadReply` for a reply the host refuses (the right stays held); otherwise the
    /// reply is handed over, or discarded when another transition ended the exchange first,
    /// and the right is released.
    fn server_reply(&mut self, args: &[Value<'p>]) -> Flow<'p> {
        let run = Rc::clone(&self.run);
        let Some(serving) = run.serve.as_ref() else {
            return Err(fault("`HttpServer.reply` in a run that serves nothing"));
        };
        serving.port.begin();
        let Some(Value::Right(right)) = args.first() else {
            return Err(fault("`HttpServer.reply` without a reply right"));
        };
        let right: Word = *right;
        let reply = reply_of(args.get(1));
        let task = self.holder();
        let mut exchange = serving.rights.take(task, right).map_err(|_| not_held())?;
        match exchange.reply(&reply) {
            Err(_) => {
                serving.rights.restore(task, right, exchange);
                Err(bad_reply())
            }
            Ok(Sent::Sent | Sent::Discarded | Sent::NotHeld) => {
                serving.rights.release(task, right);
                Ok(Value::Unit)
            }
        }
    }

    /// `start` moves to the started task `to` every right that the task that runs holds and
    /// that the captured bindings `names` reach, closures included (rule 4.2), in their order:
    /// the bindings by name, each value depth first, left to right, an allocation shared by
    /// several walked once at its first meeting. O(values reachable), each value looked at
    /// once.
    pub(super) fn move_rights(&self, captured: &Env<'p>, names: &[Name], to: Task) {
        let Some(serving) = &self.run.serve else {
            return;
        };
        if names.is_empty() {
            return;
        }
        let from = self.holder();
        let mut seen: HashSet<usize> = HashSet::new();
        let mut pending: Vec<Value<'p>> = names
            .iter()
            .rev()
            .filter_map(|name| captured.lookup(name).cloned())
            .collect();
        while let Some(value) = pending.pop() {
            let mut parts: Vec<Value<'p>> = Vec::new();
            let shared = |seen: &mut HashSet<usize>, at: usize| !seen.insert(at);
            match &value {
                Value::Right(right) => {
                    serving.rights.transfer(from, to, *right);
                }
                Value::Data(data) => {
                    if shared(&mut seen, Rc::as_ptr(data) as usize) {
                        continue;
                    }
                    parts.extend(data.fields.iter().cloned());
                }
                Value::List(items) | Value::Tuple(items) => {
                    if shared(&mut seen, Rc::as_ptr(items) as usize) {
                        continue;
                    }
                    parts.extend(items.iter().cloned());
                }
                Value::Map(entries) => {
                    if shared(&mut seen, Rc::as_ptr(entries) as usize) {
                        continue;
                    }
                    for (key, value) in entries.values() {
                        parts.push(key.clone());
                        parts.push(value.clone());
                    }
                }
                Value::Seq(items) => {
                    if shared(&mut seen, Rc::as_ptr(items) as usize) {
                        continue;
                    }
                    parts.extend(items.iter().cloned());
                }
                Value::Set(items) => {
                    if shared(&mut seen, Rc::as_ptr(items) as usize) {
                        continue;
                    }
                    parts.extend(items.values().cloned());
                }
                Value::HashMap(table) | Value::HashSet(table) => {
                    if shared(&mut seen, Rc::as_ptr(table) as usize) {
                        continue;
                    }
                    for (key, value) in table.entries.iter() {
                        parts.push(key.clone());
                        parts.push(value.clone());
                    }
                }
                Value::Priority(heap) => {
                    if shared(&mut seen, Rc::as_ptr(heap) as usize) {
                        continue;
                    }
                    for entry in &heap.entries {
                        parts.push(entry.item.clone());
                        parts.push(entry.priority.clone());
                    }
                }
                Value::Dense(dense) => {
                    if shared(&mut seen, Rc::as_ptr(&dense.buffer) as usize) {
                        continue;
                    }
                    if let Elements::Values(items) = dense.buffer.as_ref() {
                        parts.extend(items.iter().cloned());
                    }
                }
                Value::Closure(closure) => {
                    if shared(&mut seen, Rc::as_ptr(closure) as usize) {
                        continue;
                    }
                    parts.extend(closure.env.iter().map(|(_, value)| value.clone()));
                    for instance in closure.caps.values() {
                        if let super::Instance::Handled { values, .. } = instance.as_ref() {
                            parts.extend(values.iter().cloned());
                        }
                    }
                }
                _ => {}
            }
            // Depth first, left to right: the first part is walked next.
            pending.extend(parts.into_iter().rev());
        }
    }

    /// The end of the task that runs, which ended with `result`, `alone` for a started task,
    /// with the status of its auto-answers ([`status_of`], rule 4.5): each right it still holds,
    /// in the order it received them, is answered by the host, once, and appends its
    /// `unanswered` event, even when another transition of rule 4.7 ended the exchange first, so
    /// the trace does not depend on timing. Quiet code appends no event, as for its operations
    /// (S-42, `spec/26 §6` rule 4), and neither does a task that the hard stop ended
    /// ([`hard_stopped`]): native code answers every right `503` and ends the process at once
    /// there, and the interpreter writes what native code writes. O(rights held).
    pub(super) fn answer_held(&mut self, result: &Flow<'p>, alone: bool) {
        let status = status_of(result, alone);
        let run = Rc::clone(&self.run);
        let Some(serving) = run.serve.as_ref() else {
            return;
        };
        let hard = hard_stopped(result);
        if hard {
            // The engine's hard stop, made by the watcher already and here again in case the
            // task got here first: from it on the engine discards the answers below, and its
            // serve threads answer the exchanges still open, these included, `503` (rule 18
            // item 6).
            serving.port.hard_stop();
        }
        let ended = serving.rights.end(self.holder());
        let hushed = (self.hushed > 0 && !self.run.quiet_events.get()) || hard;
        for (_, mut exchange) in ended {
            exchange.answer(status);
            if !hushed {
                self.emit(Event {
                    seq: 0,
                    task: self.task.join("."),
                    instance: String::new(),
                    capability: String::new(),
                    op: String::new(),
                    args: Vec::new(),
                    outcome: EventOutcome::Unanswered(status),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{own_deadline, wake_at};

    #[test]
    fn a_wait_ends_at_the_earliest_deadline_of_any_waiter() {
        // Review of #1595, L7: the bound of a blocking wait.
        assert_eq!(wake_at(None, None), None);
        assert_eq!(wake_at(Some(10), None), Some(10));
        assert_eq!(wake_at(None, Some(7)), Some(7));
        assert_eq!(wake_at(Some(10), Some(7)), Some(7));
        assert_eq!(wake_at(Some(5), Some(7)), Some(5));
        assert_eq!(wake_at(Some(7), Some(7)), Some(7));
    }

    #[test]
    fn only_the_waiters_own_deadline_ends_its_next() {
        // Woken by its own deadline: `TimedOut`; by another's: it waits again.
        assert!(own_deadline(Some(5), wake_at(Some(5), Some(7))));
        assert!(!own_deadline(Some(10), wake_at(Some(10), Some(7))));
        assert!(!own_deadline(None, wake_at(None, Some(7))));
        assert!(own_deadline(Some(7), wake_at(Some(7), Some(7))));
        assert!(!own_deadline(None, None));
    }
}
