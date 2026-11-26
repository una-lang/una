//! The driver of a `serve` section in native code (ADR 0123 rule 16, `spec/31 §7` rule 4):
//! engine code equivalent to the program of rule 4, as the interpreter's driver is
//! (`una_interp`'s `machine::driver`), not Una source or Core IR that the package gains
//! (decision S6-D4).
//!
//! The run's main task opens a `tasks` statement whose block is a `for ever` loop: `next`, then
//! a started task per request that runs the compiled `answer` (`una_codegen`'s `serve`), which
//! calls `handle` and gives its outcome to [`answer`]: `handle`'s reply, or `500` with an empty
//! body when it failed, never the failure's payload, to `reply`, whose failure is ignored. At
//! a host stop `next` fails with `Interrupted`, the loop stops at its next iteration, and the
//! started tasks drain. The statement is bounded by the sum of the grants' `/once`
//! (`una_interp::Grant::serve_driver_bound`), so `start` waits for a place while that many
//! `handle` tasks run, even when their exchanges ended. Each operation is performed and traced as the compiled code of the
//! program of rule 4 performs it, so the bytes and the trace are that program's.
//!
//! Each request costs O(1) beside `handle` and the bytes of the request and the reply: one
//! `next`, one closure of one captured value, one `start`, which moves one reply right, and
//! one `reply`.

use crate::{ffi, host, runtime, serve, started, values};

/// The word of a stopped task (`una_codegen`'s `STOPPED`).
const STOPPED: i64 = -1;

/// How an operation of the driver ended.
enum Outcome {
    /// Its value, owned.
    Value(i64),
    /// Its failure, owned.
    Failed(i64),
    /// The task stops.
    Stopped,
}

/// An operation of `HttpServer` whose event is described at `op`, on `instance`, with the
/// borrowed arguments `args`, which `perform` performs, as the compiled code performs a host
/// operation (`Lowerer::op`): a cancelled task stops before it, and from its event on it
/// completes and keeps its own outcome.
fn operation(op: i64, instance: i64, args: &[i64], perform: impl FnOnce() -> i64) -> Outcome {
    if runtime::poll() != 0 {
        return Outcome::Stopped;
    }
    runtime::operation_begin();
    let slot = host::event(op, instance);
    for (index, &arg) in (0i64..).zip(args) {
        host::event_arg(op, slot, index, arg);
    }
    // An argument whose text does not fit the memory budget stopped the task (`spec/11 §7`,
    // #1223): the operation does not start, as in the compiled code.
    if runtime::stopped() != 0 {
        return Outcome::Stopped;
    }
    let value = perform();
    if runtime::stopped() != 0 {
        return Outcome::Stopped;
    }
    let failed = host::host_failed();
    runtime::operation_end();
    host::event_end(op, slot, value, failed);
    // So did an outcome whose text does not fit the budget, which the task does not use: the
    // value or failure the operation gave is owned here, so it is released before the task
    // stops. No outcome of the server's operations holds a large integer today.
    if runtime::stopped() != 0 {
        values::release(value);
        return Outcome::Stopped;
    }
    if failed != 0 {
        Outcome::Failed(value)
    } else {
        Outcome::Value(value)
    }
}

/// A new host instance of `HttpServer`, which the caller releases.
fn server() -> i64 {
    host::host_instance(values::text_literal("HttpServer"))
}

/// Runs the driver's `tasks` statement on the run's main task: `answer`, borrowed, is a closure
/// of the compiled `answer`, of which each started task runs a copy that holds its exchange;
/// `next` describes the event of `next`, and `site` is the driver's site. 0 when the loop ended
/// at a host stop and the started tasks drained; 2 when the task stops instead (an abort, the
/// hard stop).
pub(crate) fn drive(answer: i64, next: i64, site: i64) -> i64 {
    let at = usize::try_from(site).unwrap_or(usize::MAX);
    let Some((code, _)) = values::closure_target(answer) else {
        values::fault();
        runtime::abort_at(
            at,
            una_diag::Code::U8001,
            "internal error: the driver of the `serve` section holds no compiled `answer`"
                .to_owned(),
        );
        return 2;
    };
    // The grants' `/once` bounds the started tasks (decision S6-D6, #1724), as in the
    // interpreter: a task's place is given back when its `handle` ends.
    if started::tasks_begin_at_most(runtime::serve_driver_bound()) != 0 {
        return 2;
    }
    let instance = server();
    let mut iterations = 0;
    let stops = loop {
        // A host stop ends the loop at the start of an iteration (ADR 0116 rule 14).
        if runtime::ever_start(iterations) != 0 {
            break false;
        }
        iterations += 1;
        if runtime::budget(at) != 0 {
            break true;
        }
        match operation(next, instance, &[], || serve::next(site)) {
            Outcome::Stopped => break true,
            // `Interrupted` ends the loop at its next start; `TimedOut` and `Closed` cannot
            // happen (no deadline, no `close`).
            Outcome::Failed(failure) => values::release(failure),
            Outcome::Value(exchange) => {
                // `start answer(exchange)`: the exchange's reply right moves to the task
                // before it can run (ADR 0123 rule 4.2).
                let task = values::closure(code, 0);
                values::push(task, exchange);
                values::release(exchange);
                let status = started::start(task, at, 1, ffi::invoke);
                values::release(task);
                if status != 0 {
                    break true;
                }
            }
        }
    };
    runtime::ever_end();
    // The block ended: its tasks run to their end; it stops: they are cancelled first.
    started::tasks_end(i64::from(stops));
    values::release(instance);
    if stops { 2 } else { 0 }
}

/// The end of `answer(exchange)` in a started task, after `handle` returned `value` with the
/// status `status` (0, 1 for a failure, 2 for a stop): `reply` to the borrowed `exchange`'s
/// right, with `value` or, for a failure, `500` with an empty body, its failure ignored, so a
/// `BadReply` sends nothing and the task's end answers `500` (rule 4.5). 0, or 2 when the task
/// stops.
pub(crate) fn answer(reply: i64, site: i64, exchange: i64, value: i64, status: i64) -> i64 {
    if status == 2 || (status != 0 && value == STOPPED) {
        return 2;
    }
    let answered = if status == 0 {
        value
    } else {
        values::release(value);
        let failed = values::data(0);
        values::push(failed, (500 << 1) | 1);
        let headers = values::list();
        values::push(failed, headers);
        values::release(headers);
        values::push(failed, values::text_literal(""));
        failed
    };
    let right = values::field(exchange, 1);
    let instance = server();
    let outcome = operation(reply, instance, &[right, answered], || {
        serve::reply(site, right, answered)
    });
    values::release(instance);
    values::release(answered);
    match outcome {
        Outcome::Stopped => 2,
        Outcome::Failed(failure) => {
            values::release(failure);
            0
        }
        Outcome::Value(_) => 0,
    }
}
