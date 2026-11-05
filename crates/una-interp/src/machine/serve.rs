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

use una_core::Name;
use una_http::serve::program::{Port, Waited, peer_text};
use una_http::serve::rights::{Rights, Task, Word};
use una_http::serve::run::{Exchange, Next, Sent};
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
}

impl<'w> Serving<'w> {
    /// The program's side of the engine `port`, with no right yet.
    pub(crate) fn new(port: &'w dyn Port) -> Serving<'w> {
        Serving {
            port,
            rights: Rights::new(),
            closed: Cell::new(false),
        }
    }

    /// Answers `503` each right `task` still holds, in the order it received them, and appends
    /// no event: the task whose body never ran, or ended by a panic of the interpreter, as a
    /// stopped task (rule 4.5). O(rights held).
    pub(crate) fn answer_left(&self, task: Task) {
        for (_, mut exchange) in self.rights.end(task) {
            exchange.answer(STOPPED);
        }
    }
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
    let read = &exchange.request;
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
            Value::text(&peer_text(exchange.peer())),
            Value::text(exchange.listener()),
        ],
    )
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
            // `una run` refuses a program that serves until its engine is wired (`spec/31 §1`
            // rule 10), so only a defect reaches here.
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
        loop {
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
            // Another ready task runs first: it may answer an exchange, which frees a `/once`
            // slot, or end, which another task waits for.
            if run.yield_turn() {
                if self.cancelled() {
                    return Err(Stop::Cut(CANCEL));
                }
                continue;
            }
            let now = self.run.clock.now();
            let until = left.map(|left| {
                now.saturating_add(u64::try_from(left.as_millis()).unwrap_or(u64::MAX))
            });
            let deadline = match left {
                Some(left) if !self.run.clock.is_virtual() => {
                    std::time::Instant::now().checked_add(left)
                }
                _ => None,
            };
            let stop = std::sync::Arc::clone(&self.stop);
            match serving
                .port
                .wait(epoch, retry, deadline, &|| stop.stopped())
            {
                Waited::Changed => {}
                Waited::Stopped => return Err(interrupted()),
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

    /// The end of the task that runs, with the status of its auto-answers (rule 4.5): each
    /// right it still holds, in the order it received them, is answered by the host, once, and
    /// appends its `unanswered` event, even when another transition of rule 4.7 ended the
    /// exchange first, so the trace does not depend on timing. Quiet code appends no event, as
    /// for its operations (S-42, `spec/26 §6` rule 4). O(rights held).
    pub(super) fn answer_held(&mut self, status: u16) {
        let run = Rc::clone(&self.run);
        let Some(serving) = run.serve.as_ref() else {
            return;
        };
        let ended = serving.rights.end(self.holder());
        let hushed = self.hushed > 0 && !self.run.quiet_events.get();
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
