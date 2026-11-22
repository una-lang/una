//! The driver of a `serve` section in the interpreter (ADR 0123 rule 16, `spec/31 §7` rule 4):
//! engine code equivalent to the program of rule 4, not Una source or Core IR that the package
//! gains (decision S6-D4).
//!
//! The root task runs a `tasks` statement whose block is a `for ever` loop: `next`, then a
//! started task per request that calls `handle` with the request, turns a failure of `handle`
//! into `500` with an empty body, and gives the reply to `reply`, whose failure it ignores. At a
//! host stop `next` fails with `Interrupted`, the loop stops at its next iteration, the started
//! tasks drain, and `on_stop` runs on the root task. The statement is bounded by the sum of the
//! grants' `/once` ([`crate::Grant::serve_driver_bound`]), so `start` waits for a place while
//! that many `handle` tasks run, even when their exchanges ended. Each request costs O(1) beside `handle`
//! and the bytes of the request and the reply: one `next`, one `start`, one move of one right,
//! one `reply`.

use std::collections::BTreeMap;
use std::rc::Rc;

use una_core::{Fun, Label, Level};
use una_syntax::Span;

use super::{Caps, Flow, Instance, Machine, Request, Stop, fault, prelude_ctor};
use crate::env::Env;
use crate::value::Value;

/// The name the driver's program binds the host's `HttpServer` to.
const SERVER: &str = "server";

/// The label of the driver's values: an `Exchange` and a `Reply` are `internal` (`spec/31 §1`
/// rule 2), as are the results the program binds.
fn internal() -> Label {
    Label {
        level: Level::Internal,
        ..Label::public()
    }
}

/// `@server.<op>` of the driver's program, at `span`, with the labels its Core would carry.
fn operation<'a>(op: &'a str, labels: &'a [Label], span: Span) -> Request<'a> {
    Request {
        cap: SERVER,
        capability: "HttpServer",
        op,
        labels,
        result: internal(),
        outcome: internal(),
        span,
    }
}

/// The capabilities of the driver's program: `@server`, the host's `HttpServer`.
fn server_caps<'p>() -> Caps<'p> {
    let mut caps = BTreeMap::new();
    caps.insert(
        SERVER,
        Rc::new(Instance::Host {
            id: "host:HttpServer".to_owned(),
        }),
    );
    Rc::new(caps)
}

impl<'p, 'w> Machine<'p, 'w> {
    /// Runs the driver of a `serve` section on the root task: the requests until a host stop,
    /// then `on_stop` when the module declares it. `handle` and `on_stop` take the host
    /// capabilities of their own `uses`, as `main` does.
    pub(crate) fn serve_driver(&mut self, handle: &'p Fun, on_stop: Option<&'p Fun>) -> Flow<'p> {
        let run = Rc::clone(&self.run);
        let _root = super::serve::RootRights(&run);
        let caps = server_caps();
        // The grants' `/once` bounds the started tasks (decision S6-D6, #1724).
        let bound = crate::Grant::serve_driver_bound(self.grants, self.max_memory, self.run.cap);
        let result = self
            .tasks_with(Some(bound), |machine| machine.serve_loop(handle, &caps))
            .and_then(|_| match on_stop {
                Some(on_stop) => self.call_hosted(on_stop, Vec::new()).map(|_| Value::Unit),
                None => Ok(Value::Unit),
            });
        // The root task's end answers the reply rights it still holds (ADR 0123 rule 4.5).
        self.answer_held(&result, false);
        result
    }

    /// The `for ever` loop of the driver: until a host stop at the start of an iteration, or
    /// an abort. A failure of `next` ends the iteration only: `Interrupted` then stops the loop
    /// at its next start, and `TimedOut` and `Closed` cannot happen (no deadline, no `close`).
    fn serve_loop(&mut self, handle: &'p Fun, caps: &Caps<'p>) -> Flow<'p> {
        let span = handle.span;
        let outer = self.deadline.take();
        let mut started: u64 = 0;
        let ended = loop {
            if self.ever_start(&mut started) {
                self.stop_event();
                break Ok(());
            }
            self.run.tick_at(self.uncounted == 0);
            if let Err(stop) = self.within_budget(span) {
                break Err(stop);
            }
            self.in_ever = true;
            match self.op(caps, &operation("next", &[], span), Vec::new()) {
                Ok(exchange) => {
                    if let Err(stop) = self.start_answer(handle, exchange, caps, span) {
                        break Err(stop);
                    }
                }
                Err(Stop::Fail(_)) => {}
                Err(stop) => break Err(stop),
            }
        };
        self.in_ever = false;
        self.deadline = outer;
        ended.map(|()| Value::Unit)
    }

    /// `start answer(exchange)`: a task of the driver's statement that answers `exchange`,
    /// whose reply right moves to it before it can run (ADR 0123 rule 4.2).
    fn start_answer(
        &mut self,
        handle: &'p Fun,
        exchange: Value<'p>,
        caps: &Caps<'p>,
        span: Span,
    ) -> Result<(), Stop<'p>> {
        let Value::Data(data) = &exchange else {
            return Err(fault("`next` gave a value that is not an `Exchange`"));
        };
        let (Some(request), Some(right)) = (data.fields.first(), data.fields.get(1)) else {
            return Err(fault("`next` gave an `Exchange` without its fields"));
        };
        let (request, right) = (request.clone(), right.clone());
        // The walk of rule 4.2 over the one capture, `exchange`, which holds one right.
        let moved: Env<'p> = std::iter::once(("exchange", exchange.clone())).collect();
        let names = ["exchange".to_owned()];
        let caps = Rc::clone(caps);
        self.start_with(
            span,
            move |child| child.answer(handle, request, right, &caps, span),
            |machine, id| machine.move_rights(&moved, &names, id),
        )
    }

    /// `answer(exchange)` of the driver's program, in the started task: `handle`'s reply, or
    /// `500` with an empty body when `handle` fails, never the failure's payload; then `reply`,
    /// its failure ignored: a `BadReply` sends nothing, and the task's end answers `500`.
    fn answer(
        &mut self,
        handle: &'p Fun,
        request: Value<'p>,
        right: Value<'p>,
        caps: &Caps<'p>,
        span: Span,
    ) -> Flow<'p> {
        let reply = match self.call_hosted(handle, vec![request]) {
            Ok(reply) => reply,
            Err(Stop::Fail(_)) => Value::data(
                prelude_ctor("Reply", "Reply", 0),
                vec![
                    Value::int(500),
                    Value::List(Rc::new(Vec::new())),
                    Value::text(""),
                ],
            ),
            Err(stop) => return Err(stop),
        };
        let labels = [internal(), internal()];
        match self.op(caps, &operation("reply", &labels, span), vec![right, reply]) {
            Ok(_) | Err(Stop::Fail(_)) => Ok(Value::Unit),
            Err(stop) => Err(stop),
        }
    }
}
