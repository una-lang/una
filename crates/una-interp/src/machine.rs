//! The evaluation of Core IR: the rules of `spec/11 §4`, big-step, call-by-value, left to
//! right, with the capability environment, the world and the trace.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

use una_core::bigint::{BigInt, Sign};
use una_core::json::Halt;
use una_core::{
    Atom, Bound, CapOp, Ctor, DecimalOp, Decl, ExitTag, Expr, FixedOp, FloatOp, Fun, HandlerOp,
    IntType, Label, Level, LoopSource, MapOp, Name, Path, Prim, Program, Rhs, Source, Type, Use,
};
use una_syntax::Span;

mod dense;

use crate::clock::Clock;
use crate::host::{self, FileFault, Grant, World};
use crate::stopping::{HardStop, RunStop};
use crate::tasks::{Ending, Run, TasksScope};
use crate::value::{Closure, EQUALITY_PAIR_BYTES, Key, Value, compare, equal_within};

/// The environment `ρ`: binders and their values, innermost last. Core binders are unique
/// within a function (`spec/09 §1`), so the last binding of a name is the one in scope.
pub(crate) use crate::env::Env;

/// The capability environment `H`: bindings and their instances.
pub(crate) type Caps<'p> = Rc<BTreeMap<&'p str, Rc<Instance<'p>>>>;

/// An instance of a capability (`spec/11 §2`), with its stable id.
#[derive(Debug)]
pub(crate) enum Instance<'p> {
    /// `host(C, grant, id)`, with the id `host:<Capability>`.
    Host { id: String },
    /// `handled(h, v̄, H_h, id)`, with the id `given:<name>` or `with:<handler>#<n>`.
    Handled {
        handler: HandlerInfo<'p>,
        values: Vec<Value<'p>>,
        caps: Caps<'p>,
        id: String,
    },
    /// `limiter(at_most, per, waiting)` of `std.limit` (ADR 0117 rule 9, `spec/27 §10`): the
    /// bucket of one installation, which every task that uses the binding shares, with the id
    /// `with:limiter#<n>` or `given:<name>`.
    Limiter {
        limiter: Rc<una_http::limit::Limiter>,
        id: String,
    },
}

impl Instance<'_> {
    fn id(&self) -> &str {
        match self {
            Instance::Host { id } | Instance::Handled { id, .. } | Instance::Limiter { id, .. } => {
                id
            }
        }
    }
}

/// A handler of the program.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HandlerInfo<'p> {
    name: &'p str,
    params: &'p [(String, Type)],
    uses: &'p [Use],
    ops: &'p [HandlerOp],
}

/// One event of a trace (`spec/11 §5`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// Its position in the trace, from 1.
    pub seq: usize,
    /// The path of the task that performed it, as `2.1`, `f1.2` or `s1.3`, or empty in the
    /// root task (`spec/19 §7`, `spec/27 §6`, `§7`).
    pub task: String,
    /// The instance: `given:<name>`, `host:<Capability>` or `with:<handler>#<n>`.
    pub instance: String,
    /// The capability.
    pub capability: String,
    /// The operation.
    pub op: String,
    /// The arguments by parameter name, in parameter order, in their Una form or as
    /// `<redacted:level>`.
    pub args: Vec<(String, String)>,
    /// How the operation ended.
    pub outcome: EventOutcome,
}

/// How an operation ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventOutcome {
    /// It returned this value, in its Una form or as `<redacted:level>`.
    Ok(String),
    /// It failed.
    Fail {
        /// The variant.
        variant: String,
        /// The failure in its Una form.
        value: String,
    },
    /// It returned or failed, at an outcome label above `public` in a run's trace (S-45,
    /// #1552, `spec/11 §5`): only that label's level is written, never whether it failed, its
    /// variant or its value, so no text is built from the value.
    Redacted(String),
    /// The run aborted before it ended.
    Aborted,
    /// A task was cancelled (`spec/19 §7` rule 3): the event that ends a started task after a
    /// failed one, which names no operation. The interpreter, which runs tasks one after
    /// another, never cancels one; a native run can.
    Cancelled,
    /// A loop of the host stopped at the start of an iteration (ADR 0116 rule 14): the driver's,
    /// when the run's stop came. Like `Cancelled`, it names no operation.
    Stop,
    /// An iteration of a `for ever within` passed its deadline (ADR 0116 rule 11): the event
    /// after the first operation that the deadline made fail in the iteration. Like `Stop`, it
    /// names no operation. Also the event of a `within` that timed out, after the outcomes of
    /// the operations it interrupted (ADR 0117 rule 5).
    Timeout,
    /// An interruptible operation, one of `Http`, was waiting when its task was cancelled: the
    /// loser of a `first of` (ADR 0117 rules 4 and 10). It may have reached its server, an
    /// effect in doubt. The interpreter, which runs branches one after another, never
    /// interrupts one; a native run can.
    Interrupted,
    /// A started task ran too deep, past `--task-depth` or short of its stack, and ended alone
    /// (`spec/27 §7`, ADR 0117 rules 7 and 10). Like `Cancelled`, it names no operation.
    TaskAborted,
    /// A task ended holding a reply right, which the host answered with this status, `500` or
    /// `503` (ADR 0123 rule 4.5): appended whether or not the answer was sent, so the trace does
    /// not depend on timing. Like `Cancelled`, it names no operation.
    Unanswered(u16),
}

/// The deadline of the iteration of a `for ever within` in progress (ADR 0116 rule 11), on the
/// run's clock: under virtual time, only the virtual clock passes it (rule 19, `spec/26 §8`).
#[derive(Clone, Debug)]
pub(crate) struct Deadline {
    /// When it passes, in milliseconds of the run's clock; `None` when it is beyond what the
    /// clock can hold.
    pub(crate) at: Option<u64>,
    /// The path of the task that runs the loop, whose control flow observes the deadline's
    /// failures (`spec/26 §7`).
    pub(crate) owner: Vec<String>,
    /// Whether a task the iteration started met the deadline, for the join of its block.
    pub(crate) hit: bool,
    /// Whether the iteration's `timeout` event is in the trace already.
    pub(crate) reported: bool,
}

impl Deadline {
    /// The deadline `ms` milliseconds from now on `clock`, of a loop run by the task at
    /// `owner`.
    pub(crate) fn after(ms: u64, owner: Vec<String>, clock: &Clock) -> Deadline {
        Deadline {
            at: clock.now().checked_add(ms),
            owner,
            hit: false,
            reported: false,
        }
    }

    /// The time left before it on `clock`, zero once it has passed; `None` for a deadline
    /// that never passes.
    pub(crate) fn left(&self, clock: &Clock) -> Option<std::time::Duration> {
        self.at
            .map(|at| std::time::Duration::from_millis(at.saturating_sub(clock.now())))
    }
}

/// Why a run stops without a value or a failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Abort {
    /// The call at this span exceeds the maximum depth (U6003, `spec/11 §7`).
    Depth {
        /// The call.
        span: Span,
        /// The maximum depth.
        limit: usize,
    },
    /// The call at this span finds the run over its memory budget (U6003, `spec/11 §7`,
    /// ADR 0061).
    Memory {
        /// The call.
        span: Span,
        /// The budget, in MiB.
        limit: usize,
    },
    /// The operation of `Http` at this span is given a header value with a control character
    /// (U6004, `spec/24 §1`).
    HeaderValue {
        /// The operation.
        span: Span,
    },
    /// The operation at this span made a text from host bytes that are not valid UTF-8: a
    /// file's content or an environment variable's value (U6005, S-16, ADR 0086).
    NotText {
        /// The operation.
        span: Span,
        /// The message, which names what the bytes were.
        message: &'static str,
    },
    /// The host gave no random bytes for the process key of keyed hashing, before the run
    /// started (U6009, `spec/28 §1` rule 3, ADR 0119 rule 9).
    NoRandomBytes,
    /// The call at this span makes a hash table after the run's table counter reached its
    /// limit (U6003, `spec/28 §1` rule 4).
    Tables {
        /// The call.
        span: Span,
    },
    /// The hard stop of the host came at the check point at this span: a call or a loop
    /// iteration (U6007, ADR 0116 rule 16).
    HostStop {
        /// The check point.
        span: Span,
        /// Why the hard stop came.
        hard: HardStop,
    },
    /// The `start` at this span could not start its task (U6003, `spec/27 §7`): no stack could
    /// be mapped for it, or, with `cap`, the run already holds as many started tasks as this
    /// system allows at once (ADR 0117 rule 2). Without a span, the root task of a program with
    /// a `tasks` statement could not get its stack.
    Unstarted {
        /// The `start`, if any.
        span: Option<Span>,
        /// The most started tasks the system allows at once, when that is why.
        cap: Option<usize>,
    },
    /// A fault that a checked and verified program cannot reach, such as a division by zero
    /// while proof obligations are not verified yet (ADR 0027).
    Fault {
        /// What happened.
        message: String,
        /// The name of the function it happened in, when there is one.
        span: Option<Span>,
    },
}

/// Why evaluation stops early: a failure, which `handle` and `attempt` catch, or an abort.
pub(crate) enum Stop<'p> {
    /// `fail(C(v, ...))`.
    Fail(Value<'p>),
    /// An abort.
    Abort(Abort),
    /// `exit_k(b, ...)`: the end of an iteration of the innermost loop, by `skip` or `stop`,
    /// with the loop's state (`spec/26 §3`); the loop catches it.
    Exit(ExitTag, Vec<Value<'p>>),
    /// The deadline of the `within` with this id passed: its expression is cancelled at a
    /// check point, and nothing inside runs further, until that `within` catches it and fails
    /// with its time-out (`spec/27 §5`, ADR 0117 rule 5).
    Cut(u64),
}

/// The result of evaluating an expression.
pub(crate) type Flow<'p> = Result<Value<'p>, Stop<'p>>;

mod priority;

mod seqs;

mod sets;

pub(crate) mod hashes;

pub(crate) mod serve;

mod driver;

mod bitsets;

/// The fault of a `slice` outside the text (#693).
const SLICE_OUTSIDE: &str = "`slice` outside the text, which its precondition excludes";

/// The fault of a prelude call whose `requires` does not hold, at the call `span`, where native
/// code aborts too (#480, #693).
fn precondition<'p>(message: &str, span: Span) -> Stop<'p> {
    Stop::Abort(Abort::Fault {
        message: message.to_owned(),
        span: Some(span),
    })
}

/// An `Http` request as owned data, which a helper thread can take (#1730).
struct Owned {
    method: una_http::grant::Method,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    bearer: Option<Vec<u8>>,
}

/// Sends `owned` under the run's limits `governor`, interrupted by the host stop `stop` when
/// given: the request is in the body of a `for ever` (ADR 0116 rule 12).
fn send(
    owned: &Owned,
    governor: &una_http::limit::Governor,
    settings: &una_http::client::Settings,
    stop: Option<&RunStop>,
    cover: Option<u64>,
) -> Result<una_http::client::Response, una_http::client::Error> {
    use una_http::client::send_governed;
    // Nothing abandons a request of the interpreter: a cancellation leaves it to its deadline
    // (#1811).
    let abandon = &una_http::limit::Abandon::new();
    let request = una_http::client::Request {
        method: owned.method,
        url: &owned.url,
        headers: &owned.headers,
        body: &owned.body,
        bearer: owned.bearer.as_deref(),
    };
    match stop {
        Some(stop) => send_governed(
            &request,
            governor,
            settings,
            &stop.interrupt(),
            abandon,
            cover,
        ),
        None => send_governed(
            &request,
            governor,
            settings,
            &una_http::client::Uninterrupted,
            abandon,
            cover,
        ),
    }
}

/// Rings [`crate::tasks::Landed`] for a task when dropped, at the end of its helper.
struct Ring(
    Arc<crate::tasks::Landed>,
    usize,
    Arc<std::sync::atomic::AtomicBool>,
);

impl Drop for Ring {
    fn drop(&mut self) {
        self.0.ring(self.1, &self.2);
    }
}

fn fault<'p>(message: impl Into<String>) -> Stop<'p> {
    Stop::Abort(Abort::Fault {
        message: message.into(),
        span: None,
    })
}

/// A qualified name: a module path and a declaration's name.
type Qualified<'p> = (&'p str, &'p str);

/// A predicate: its parameters and its body.
type Predicate<'p> = (&'p [(String, Type)], &'p Expr);

/// The fields of each variant of a kind, by name, with their declared types.
type Variants<'p> = Vec<Vec<(&'p str, &'p Type)>>;

/// The declarations of a program, by qualified name.
pub(crate) struct Index<'p> {
    functions: BTreeMap<Qualified<'p>, &'p Fun>,
    predicates: BTreeMap<Qualified<'p>, Predicate<'p>>,
    handlers: BTreeMap<Qualified<'p>, HandlerInfo<'p>>,
    capabilities: BTreeMap<&'p str, &'p [CapOp]>,
    data: BTreeMap<Qualified<'p>, Variants<'p>>,
    /// Whether the program has a `tasks` statement: its runs run their root task as a
    /// coroutine (ADR 0117 rule 11).
    pub(crate) tasks: bool,
}

impl<'p> Index<'p> {
    /// Indexes the declarations of `program`.
    pub(crate) fn new(program: &'p Program) -> Index<'p> {
        let mut index = Index {
            functions: BTreeMap::new(),
            predicates: BTreeMap::new(),
            handlers: BTreeMap::new(),
            capabilities: BTreeMap::new(),
            data: BTreeMap::new(),
            tasks: una_core::has_tasks(program),
        };
        for module in &program.modules {
            let path = module.path.as_str();
            for decl in &module.decls {
                match decl {
                    Decl::Fun(fun) => {
                        index.functions.insert((path, fun.name.as_str()), fun);
                    }
                    Decl::Pred {
                        name, params, body, ..
                    } => {
                        index
                            .predicates
                            .insert((path, name.as_str()), (params.as_slice(), body));
                    }
                    Decl::Handler {
                        name,
                        params,
                        uses,
                        ops,
                        ..
                    } => {
                        let info = HandlerInfo {
                            name,
                            params,
                            uses,
                            ops,
                        };
                        index.handlers.insert((path, name.as_str()), info);
                    }
                    Decl::Cap { name, ops } => {
                        index.capabilities.entry(name.as_str()).or_insert(ops);
                    }
                    Decl::Data { name, variants, .. } => {
                        let fields = variants
                            .iter()
                            .map(|v| v.fields.iter().map(|(f, t)| (f.as_str(), t)).collect())
                            .collect();
                        index.data.insert((path, name.as_str()), fields);
                    }
                    Decl::Alias { .. } | Decl::Policy { .. } => {}
                }
            }
        }
        index
    }

    /// The function `name` of module `module`.
    pub(crate) fn function(&self, module: &str, name: &str) -> Option<&'p Fun> {
        self.functions.get(&(module, name)).copied()
    }

    /// The names of the fields of `ctor`, with their declared types when they are known: a
    /// kind that Core declares, the program's or the prelude's as `Header` (`spec/24 §1`), by
    /// its declaration, so that its labeled fields are redacted as the executable redacts them.
    /// A kind of the prelude that Core does not declare has its fields' names from
    /// [`una_core::prelude_variants`], as the native backend's descriptions do, and no
    /// declared type, since none of them declares a label (#415).
    fn fields(&self, ctor: &Ctor) -> Vec<(&'p str, Option<&'p Type>)> {
        let declared = self
            .data
            .get(&(ctor.kind.module.as_str(), ctor.kind.name.as_str()))
            .and_then(|variants| variants.get(ctor.index));
        if let Some(fields) = declared {
            return fields.iter().map(|(f, t)| (*f, Some(*t))).collect();
        }
        if ctor.kind.module == una_core::PRELUDE_MODULE
            && let Some((_, fields)) = una_core::prelude_variants(&ctor.kind.name)
                .and_then(|variants| variants.iter().find(|(name, _)| *name == ctor.name))
        {
            return fields.iter().map(|(name, _)| (*name, None)).collect();
        }
        Vec::new()
    }

    /// The Una form of `value`. A field whose declared type is labeled above
    /// `public` is `<redacted:level>`. `room` is asked before a large `Int` is written
    /// ([`Room`]).
    pub(crate) fn render(&self, value: &Value<'_>, room: Room<'_>) -> Written {
        self.una_form(value, Form::Redacted, room)
    }

    /// The Una form of `value` with every field, which interpolation writes: its text carries
    /// the value's labels (`spec/11 §5`, ADR 0068).
    pub(crate) fn written(&self, value: &Value<'_>, room: Room<'_>) -> Written {
        self.una_form(value, Form::Written, room)
    }

    /// The Una form of `value` in `form`: an example's results and reasons, which edition `0.2`
    /// shows with every field (`spec/11 §5`, ADR 0078).
    pub(crate) fn shown(&self, value: &Value<'_>, form: Form, room: Room<'_>) -> Written {
        self.una_form(value, form, room)
    }

    /// The Una form of `value` (`spec/11 §5`), in `form`. An `Int` of any size is written, in
    /// O(M(d) log d) for its d digits, once `room` has the bytes of a large one (`spec/11 §7`,
    /// #1223).
    fn una_form(&self, value: &Value<'_>, form: Form, room: Room<'_>) -> Written {
        self.una_form_in(value, form, None, room)
    }

    /// The Una form of `value` in an event of the host's `HttpServer`, in `form`, with the
    /// fields of the server's kinds in the reduced form of ADR 0123 rule 1 under `key`
    /// ([`crate::serve_trace`]).
    pub(crate) fn server_form(
        &self,
        value: &Value<'_>,
        form: Form,
        key: &una_http::serve::trace::TraceKey,
        room: Room<'_>,
    ) -> Written {
        self.una_form_in(value, form, Some(key), room)
    }

    /// `value` in an event of the host's `HttpServer`, redacted by `label` as [`Self::redacted`]
    /// redacts, and otherwise in the reduced form of ADR 0123 rule 1 under `key`.
    pub(crate) fn server_redacted(
        &self,
        value: &Value<'_>,
        label: &Label,
        key: &una_http::serve::trace::TraceKey,
        room: Room<'_>,
    ) -> Written {
        if label.is_public() || matches!(value, Value::Unit) {
            self.server_form(value, Form::Redacted, key, room)
        } else {
            Ok(format!("<redacted:{}>", level_name(label.level)))
        }
    }

    /// [`Self::una_form`], with the reduced form of the server's kinds under `reduce`.
    fn una_form_in(
        &self,
        value: &Value<'_>,
        form: Form,
        reduce: Option<&una_http::serve::trace::TraceKey>,
        room: Room<'_>,
    ) -> Written {
        // A worklist instead of recursion: a value built by a loop can be deeper than the
        // native stack allows (`spec/17 §4`, ADR 0032).
        enum Task<'v, 'p> {
            Value(&'v Value<'p>),
            Text(String),
        }
        let mut out = String::new();
        let mut pending = vec![Task::Value(value)];
        while let Some(task) = pending.pop() {
            let value = match task {
                Task::Text(text) => {
                    put(&mut out, &text, room)?;
                    continue;
                }
                Task::Value(value) => value,
            };
            let mut parts: Vec<Task<'_, '_>> = Vec::new();
            match value {
                Value::Int(n) => {
                    if una_core::int_text_room(n).is_some_and(|bytes| !room(bytes)) {
                        return Err(OverBudget);
                    }
                    put(&mut out, &n.to_string(), room)?;
                }
                Value::F64(x) => put(&mut out, &una_core::float::render(*x), room)?,
                Value::Decimal(c, s) => {
                    if una_core::int_text_room(c).is_some_and(|bytes| !room(bytes)) {
                        return Err(OverBudget);
                    }
                    put(&mut out, &una_core::decimal::render(c, *s), room)?
                }
                Value::F32(x) => put(&mut out, &una_core::float32::render(*x), room)?,
                Value::Bool(b) => put(&mut out, &b.to_string(), room)?,
                Value::Text(t) => put(&mut out, &una_core::quote(t), room)?,
                Value::Unit => put(&mut out, "unit", room)?,
                // Interpolation writes no bytes of a hash (ADR 0068, amendment of
                // 2026-10-06); traces, examples and diagnostics show them.
                Value::Hashed(_) if form == Form::Written => put(&mut out, "<hashed>", room)?,
                Value::Hashed(bytes) => {
                    put(&mut out, &format!("hashed:{}", host::hex(bytes)), room)?
                }
                Value::Closure(_) => put(&mut out, "<function>", room)?,
                // Nothing of a reply right is ever written (ADR 0123 rule 1).
                Value::Right(_) => put(&mut out, "<reply-right>", room)?,
                Value::List(items) | Value::Tuple(items) => {
                    let (open, close) = if matches!(value, Value::List(_)) {
                        ("[", "]")
                    } else {
                        ("(", ")")
                    };
                    parts.push(Task::Text(open.to_owned()));
                    for (i, item) in items.iter().enumerate() {
                        if i > 0 {
                            parts.push(Task::Text(", ".to_owned()));
                        }
                        parts.push(Task::Value(item));
                    }
                    parts.push(Task::Text(close.to_owned()));
                }
                // An `Lu[T]` renders as `<lu size n>`, which is not source, as `<hashed>` is not
                // (ADR 0120 rule 38).
                Value::Dense(dense) if dense.kind == crate::value::DenseKind::Lu => {
                    let size = dense.shape.get(1).copied().unwrap_or(0);
                    put(&mut out, &format!("<lu size {size}>"), room)?;
                }
                // The construction call that rebuilds a dense value, its elements in
                // row-major order (ADR 0120 rule 41), each a task like a list's, so a deep
                // value never recurses on the native stack.
                Value::Dense(dense) => {
                    let open = match (dense.kind, dense.shape.as_slice()) {
                        (crate::value::DenseKind::Matrix, [rows, columns]) => {
                            format!("matrix(rows: {rows}, columns: {columns}, elements: [")
                        }
                        (crate::value::DenseKind::Tensor, shape) => {
                            let lengths: Vec<String> =
                                shape.iter().map(ToString::to_string).collect();
                            format!("tensor(shape: [{}], elements: [", lengths.join(", "))
                        }
                        _ => "array([".to_owned(),
                    };
                    parts.push(Task::Text(open));
                    for k in 0..dense.count() {
                        if k > 0 {
                            parts.push(Task::Text(", ".to_owned()));
                        }
                        let Some(position) = dense.position_at(k) else {
                            continue;
                        };
                        match dense.buffer.as_ref() {
                            crate::value::Elements::F64(items) => {
                                if let Some(x) = items.get(position) {
                                    parts.push(Task::Text(una_core::float::render(*x)));
                                }
                            }
                            crate::value::Elements::Values(items) => {
                                if let Some(item) = items.get(position) {
                                    parts.push(Task::Value(item));
                                }
                            }
                        }
                    }
                    parts.push(Task::Text("])".to_owned()));
                }
                // `{k: v, ...}` in key order, `{}` when empty (`spec/21 §2` rule 4).
                Value::Map(entries) => {
                    parts.push(Task::Text("{".to_owned()));
                    for (i, (key, value)) in entries.values().enumerate() {
                        if i > 0 {
                            parts.push(Task::Text(", ".to_owned()));
                        }
                        parts.push(Task::Value(key));
                        parts.push(Task::Text(": ".to_owned()));
                        parts.push(Task::Value(value));
                    }
                    parts.push(Task::Text("}".to_owned()));
                }
                // A `Stack` or a `Queue` as a list, front first (`spec/28 §4`, ADR 0119 rule 3).
                Value::Seq(items) => {
                    parts.push(Task::Text("[".to_owned()));
                    for (i, item) in items.iter().enumerate() {
                        if i > 0 {
                            parts.push(Task::Text(", ".to_owned()));
                        }
                        parts.push(Task::Value(item));
                    }
                    parts.push(Task::Text("]".to_owned()));
                }
                // `{1, 2, 3}` in ascending order, `{}` when empty (`spec/28 §4`, ADR 0119
                // rule 3).
                Value::Set(items) => {
                    parts.push(Task::Text("{".to_owned()));
                    for (i, item) in items.values().enumerate() {
                        if i > 0 {
                            parts.push(Task::Text(", ".to_owned()));
                        }
                        parts.push(Task::Value(item));
                    }
                    parts.push(Task::Text("}".to_owned()));
                }
                // `{k: v, ...}` and `{a, b}` in insertion order, `{}` when empty (`spec/28 §4`,
                // ADR 0119 rules 3 and 10): never in the order of the hashes.
                Value::HashMap(table) => {
                    parts.push(Task::Text("{".to_owned()));
                    for (i, (key, value)) in table.entries.iter().enumerate() {
                        if i > 0 {
                            parts.push(Task::Text(", ".to_owned()));
                        }
                        parts.push(Task::Value(key));
                        parts.push(Task::Text(": ".to_owned()));
                        parts.push(Task::Value(value));
                    }
                    parts.push(Task::Text("}".to_owned()));
                }
                Value::HashSet(table) => {
                    parts.push(Task::Text("{".to_owned()));
                    for (i, (item, _)) in table.entries.iter().enumerate() {
                        if i > 0 {
                            parts.push(Task::Text(", ".to_owned()));
                        }
                        parts.push(Task::Value(item));
                    }
                    parts.push(Task::Text("}".to_owned()));
                }
                // A `PriorityQueue` as a list, in the order of its pops (`spec/28 §4`, ADR 0119
                // rule 3): O(n log n) to order its entries.
                Value::Priority(heap) => {
                    parts.push(Task::Text("[".to_owned()));
                    for (i, entry) in priority::listed(heap).into_iter().enumerate() {
                        if i > 0 {
                            parts.push(Task::Text(", ".to_owned()));
                        }
                        parts.push(Task::Value(&entry.item));
                    }
                    parts.push(Task::Text("]".to_owned()));
                }
                // A bit set's members in braces, ascending, `{}` when empty (`spec/28 §4`, ADR
                // 0119 rule 3): integers, written at once.
                Value::BitSet(bits) => {
                    let members: Vec<String> =
                        bits.members().iter().map(ToString::to_string).collect();
                    parts.push(Task::Text(format!("{{{}}}", members.join(", "))));
                }
                Value::Data(data) => {
                    let names = self.fields(&data.ctor);
                    // A complex value is written `complex(re: ..., im: ...)` (ADR 0120 rule 24).
                    let constructor = una_core::complex::constructor_name(&data.ctor);
                    if data.fields.is_empty() {
                        put(&mut out, constructor, room)?;
                        continue;
                    }
                    parts.push(Task::Text(format!("{constructor}(")));
                    let named = data.fields.len() > 1;
                    // In an event of the host's `HttpServer`, the fields of its kinds that the
                    // reduced form of ADR 0123 rule 1 changes (`serve_trace`).
                    let server = reduce.filter(|_| {
                        data.ctor.kind.module == una_core::PRELUDE_MODULE
                            && crate::serve_trace::is_server_kind(&data.ctor.kind.name)
                    });
                    let header = match data.fields.first() {
                        Some(Value::Text(name)) => Some(name.as_str()),
                        _ => None,
                    };
                    for (i, field) in data.fields.iter().enumerate() {
                        let (name, ty) = names.get(i).copied().unwrap_or(("?", None));
                        if i > 0 {
                            parts.push(Task::Text(", ".to_owned()));
                        }
                        if named {
                            parts.push(Task::Text(format!("{name}: ")));
                        }
                        let label = ty.map_or_else(Label::public, Type::deep_label);
                        let reduced = server.and_then(|key| {
                            let how = crate::serve_trace::field(&data.ctor.kind.name, i, header)?;
                            match field {
                                Value::Text(text) => {
                                    Some(crate::serve_trace::write(how, key, text.as_str()))
                                }
                                // These fields are `Text`; any other value is written as
                                // nothing of it, never in clear.
                                _ => Some("?".to_owned()),
                            }
                        });
                        if form != Form::Redacted
                            || label.is_public()
                            || matches!(field, Value::Unit)
                        {
                            match reduced {
                                Some(text) => parts.push(Task::Text(text)),
                                None => parts.push(Task::Value(field)),
                            }
                        } else {
                            parts.push(Task::Text(format!(
                                "<redacted:{}>",
                                level_name(label.level)
                            )));
                        }
                    }
                    parts.push(Task::Text(")".to_owned()));
                }
            }
            pending.extend(parts.into_iter().rev());
        }
        Ok(out)
    }

    pub(crate) fn redacted(&self, value: &Value<'_>, label: &Label, room: Room<'_>) -> Written {
        if label.is_public() || matches!(value, Value::Unit) {
            self.render(value, room)
        } else {
            Ok(format!("<redacted:{}>", level_name(label.level)))
        }
    }

    /// The variant of a failure and its fields whose declared labels are `public`, in their Una
    /// form (`spec/04 §8`). A field of a type parameter is not shown, since its label is not
    /// known at run time.
    /// A field whose text does not fit the budget is named, [`OVER_BUDGET`].
    pub(crate) fn public_fields(&self, value: &Value<'_>, room: Room<'_>) -> Vec<(String, String)> {
        let Value::Data(data) = value else {
            return Vec::new();
        };
        let names = self.fields(&data.ctor);
        data.fields
            .iter()
            .zip(names)
            .filter(|(_, (_, ty))| {
                ty.is_some_and(|ty| !ty.has_param() && ty.deep_label().is_public())
            })
            .map(|(v, (name, _))| {
                let text = self
                    .render(v, room)
                    .unwrap_or_else(|OverBudget| OVER_BUDGET.to_owned());
                (name.to_owned(), text)
            })
            .collect()
    }
}

/// Whether a write may hold `bytes` more within the run's memory budget (ADR 0061): asked
/// before an `Int` of at least [`una_core::INT_TEXT_RESERVED_FROM`] bytes of text and
/// temporaries is converted, so that the write never passes the hard ceiling (`spec/11 §7`,
/// #1223). [`Machine::room`] gives the run's.
pub(crate) type Room<'a> = &'a dyn Fn(usize) -> bool;

/// Appends `piece` to `out`, which first grows, when it must, within the memory budget: a
/// growth to at least [`una_core::INT_TEXT_RESERVED_FROM`] bytes asks `room` for the new
/// buffer and the piece (`spec/11 §7`, #1223), so a text built of many small parts is checked
/// as it grows, at O(1) amortized per part.
fn put(out: &mut String, piece: &str, room: Room<'_>) -> Result<(), OverBudget> {
    let need = out.len().saturating_add(piece.len());
    if need > out.capacity() {
        let grown = need.max(out.capacity().saturating_mul(2));
        let held = grown.saturating_add(piece.len());
        if held >= una_core::INT_TEXT_RESERVED_FROM && !room(held) {
            return Err(OverBudget);
        }
        out.reserve_exact(grown - out.len());
    }
    out.push_str(piece);
    Ok(())
}

/// What a message (U6001, a test's report) says of a value whose text does not fit the run's
/// memory budget (`spec/11 §7`, #1223): it is named rather than written.
pub(crate) const OVER_BUDGET: &str = "<a value whose text exceeds the memory budget>";

/// A value's text, or [`OverBudget`].
pub(crate) type Written = Result<String, OverBudget>;

/// A write that does not fit the run's memory budget: the run aborts with U6003 at its site
/// (`spec/11 §7`, #1223).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OverBudget;

fn level_name(level: Level) -> &'static str {
    match level {
        Level::Public => "public",
        Level::Internal => "internal",
        Level::Confidential => "confidential",
        Level::Secret => "secret",
        Level::Restricted => "restricted",
    }
}

pub(crate) fn prelude_ctor(kind: &str, name: &str, index: usize) -> Ctor {
    Ctor {
        kind: Path {
            module: "prelude".to_owned(),
            name: kind.to_owned(),
        },
        name: name.to_owned(),
        index,
    }
}

fn some(value: Value<'_>) -> Value<'_> {
    Value::data(prelude_ctor("Option", "Some", 0), vec![value])
}

/// The failure `RateLimited` of `Limited` of `std.limit` (ADR 0117 rule 9, `spec/27 §10`).
fn rate_limited<'p>() -> Stop<'p> {
    Stop::Fail(Value::data(
        Ctor {
            kind: Path {
                module: una_core::LIMIT_MODULE.to_owned(),
                name: "Limited".to_owned(),
            },
            name: "RateLimited".to_owned(),
            index: 0,
        },
        Vec::new(),
    ))
}

fn none<'p>() -> Value<'p> {
    Value::data(prelude_ctor("Option", "None", 1), Vec::new())
}

/// A binder's source name: Core renames a binder that shares a name with another, and `k%2`
/// is keyed as `k` in traces (`spec/11 §5`).
fn source_name(name: &str) -> String {
    match name.split_once('%') {
        Some((source, _)) if !source.is_empty() => source.to_owned(),
        _ => name.to_owned(),
    }
}

/// The parameter names of the operations of host capabilities (`spec/14 §3`).
fn host_params(capability: &str, op: &str) -> &'static [&'static str] {
    match (capability, op) {
        ("Console", "print") => &["line"],
        ("Env", "get") => &["name"],
        ("Files", "read") => &["path"],
        ("Files", "write") => &["path", "content"],
        ("Keys", "hmac") => &["key_name", "data"],
        ("Http", "get" | "delete") => &["url", "headers", "key_name"],
        ("HttpServer", "reply") => &["to", "reply"],
        ("Http", "post" | "put") => &["url", "headers", "body", "key_name"],
        _ => &[],
    }
}

/// How a value's Una form treats labeled fields and hashes (`spec/11 §5`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Form {
    /// Traces, and the examples of edition `0.1`: a field whose declared type is labeled above
    /// `public` is `<redacted:level>`, and a hash shows its bytes.
    Redacted,
    /// Interpolation: every field, and no bytes of a hash (ADR 0068).
    Written,
    /// The examples of edition `0.2`: every field, and a hash shows its bytes, since their data
    /// come from the source (ADR 0078).
    Shown,
}

/// The host-driven step's settings (ADR 0116 rules 1 and 3, `spec/26 §2`): the tick period and
/// the step budget in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drive {
    /// The tick period, from 1 millisecond.
    pub ticks: u64,
    /// The step budget, when one is set.
    pub budget: Option<u64>,
}

/// The limits of a run (`spec/11 §7`): its maximum call depth, and its memory budget, in MiB,
/// with a probe of the bytes the process holds on the heap (ADR 0061). The machine never
/// touches the allocator: the binary that counts allocations passes the probe.
#[derive(Clone, Copy)]
pub struct Limits<'a> {
    /// The maximum call depth.
    pub max_depth: usize,
    /// The memory budget, in MiB.
    pub max_memory: usize,
    /// The bytes the process holds on the heap now.
    pub held: &'a (dyn Fn() -> usize + Sync),
    /// The limits and deadline of each HTTP request (`spec/24 §4`).
    pub http: una_http::client::Settings,
    /// `--stop-after-iterations`: the budget of each loop instance, a `for ever` entered or the
    /// driver's loop, which stops at the start of its iteration `n + 1` (ADR 0116 rule 19,
    /// `spec/26 §6`, #1472).
    pub stop_after: Option<u64>,
    /// `--task-depth`: the calls a branch of `first of` may make below its block, within
    /// `max_depth` (`spec/27 §3`, ADR 0117 rule 2).
    pub task_depth: usize,
    /// The run's host stop state, which its watcher shares (ADR 0116 rules 14 and 16); a run
    /// without one makes its own, which only its own stop reaches.
    pub stop: Option<&'a Arc<RunStop>>,
    /// The table of dense kernels the run calls (ADR 0120 rules 53 and 56): the baseline by
    /// default; the caller may give the variant its processor runs, which gives the same bits.
    pub kernels: &'static una_core::dense::variants::Kernels,
    /// The serve engine of a run that serves (ADR 0123 rules 1 to 4): the operations of
    /// `HttpServer` meet it. `None` in a run that serves nothing, where such an operation is a
    /// fault, since `una run` refuses the program.
    pub serve: Option<&'a dyn una_http::serve::program::Port>,
    /// The places for the sockets of the `Http` requests that a run serving live peers makes
    /// off its thread (`spec/27 §2` rule 3); `None` takes them from the soft limit of open
    /// descriptors, as native code does. The toolchain's tests set fewer.
    pub http_places: Option<usize>,
}

impl Limits<'_> {
    /// The depth `max_depth`, with a memory budget nothing measures: for a caller that does not
    /// count allocations.
    pub fn depth(max_depth: usize) -> Limits<'static> {
        Limits {
            max_depth,
            max_memory: usize::MAX,
            held: &unmeasured,
            http: una_http::client::Settings::default(),
            stop_after: None,
            task_depth: max_depth,
            stop: None,
            kernels: &una_core::dense::variants::Kernels::BASELINE,
            serve: None,
            http_places: None,
        }
    }
}

fn unmeasured() -> usize {
    0
}

/// What a loop visits, its variable, its state, its body and where it is written.
type LoopParts<'p> = (
    &'p LoopSource,
    &'p (Name, Type),
    &'p [(Name, Type)],
    &'p Expr,
    Span,
);

/// One branch of a `together`, `together for` or `first of` whose events the task holds until
/// its block's join (`spec/19 §7`, ADR 0117 rule 10): where its events start, its path, and how
/// many instances of each handler it installed, whose ids `with:<handler>#@<path>#<k>` the join
/// renumbers, as native code's join does.
#[derive(Clone, Debug, Default)]
pub(crate) struct Buffer {
    start: usize,
    path: String,
    installs: BTreeMap<String, usize>,
}

/// A branch that ended, for its block's join: its buffer, and where its events end.
#[derive(Clone, Debug)]
pub(crate) struct Part {
    buffer: Buffer,
    end: usize,
}

/// The id a cut carries when its task is cancelled rather than one of its `within`s timed out:
/// no `within` has it, so nothing catches it (`spec/27 §7`).
const CANCEL: u64 = u64::MAX;

/// The Una Abstract Machine: one task of a run of `main` or of one example. The state of the
/// run that its tasks share is in [`Run`] (ADR 0117 rule 11).
pub(crate) struct Machine<'p, 'w> {
    index: &'w Index<'p>,
    grants: &'w [Grant],
    keys: &'w BTreeMap<String, Vec<u8>>,
    /// The limits and deadline of each HTTP request.
    http: una_http::client::Settings,
    /// The state of the run that its tasks share.
    pub(crate) run: Rc<Run<'p, 'w>>,
    depth: usize,
    max_depth: usize,
    /// The run's host stop state (ADR 0116 rules 14 and 16).
    stop: Arc<RunStop>,
    /// Whether evaluation is in the body of a `for ever`, in that body's own task, callees
    /// included: its `Http` operations are interruptible there (rule 14). No `for ever` holds
    /// another one, so a flag suffices.
    in_ever: bool,
    /// How many `quiet` expressions the task is in, those of the task that started it at its
    /// `start` included: while one is, a `for ever` that a host stop ends appends no `stop`
    /// event (S-39, `spec/26 §6` rule 4).
    quiet: u32,
    /// How many of those are not quiet by a timing floor alone, counted the same way: while
    /// one is, an operation appends no event (S-42, #1515, `spec/26 §6` rule 4).
    hushed: u32,
    /// The thread's attention word, and its value at the last check point (rule 18).
    word: una_sys::attention::Word,
    seen: Cell<u64>,
    /// The memory budget in MiB, the probe of the heap, and what the heap held when the run
    /// started, which does not count.
    max_memory: usize,
    held: &'w dyn Fn() -> usize,
    baseline: usize,
    /// Where the evaluator's stack starts, and how much of it evaluation may use.
    pub(crate) stack_base: usize,
    stack_budget: usize,
    /// The name spans of the functions in progress, innermost last, to locate faults.
    frames: Vec<Span>,
    /// The path of the task in progress, one part per enclosing block (`spec/19 §6`): the
    /// 1-based index of a `together` task; for a branch of the `c`-th `first of` its task runs,
    /// `f<c>` and the branch's 1-based index (`spec/27 §6`); for the `k`-th task of its task's
    /// `c`-th `tasks` statement, `s<c>` and `k` (`spec/27 §7`); `qf`, `qs` and `q`, with no
    /// number, for the blocks and tasks of quiet code, which are not counted (S-39, S-40).
    task: Vec<String>,
    /// How many `first of` blocks each task in progress has run, the root's first, for the
    /// paths of their branches (`spec/27 §6`): outside `quiet` code only, so that quiet code
    /// never moves the paths of later public blocks (S-39, `spec/26 §6` rule 4).
    races: Vec<crate::concurrency::BlockCounts>,
    /// How many `tasks` statements each task in progress has run, for the paths of the tasks
    /// they start (`spec/27 §7`), outside `quiet` code only, as `races`.
    tasks_runs: Vec<crate::concurrency::BlockCounts>,
    /// `--task-depth`: the calls a branch of `first of`, or a started task, may make below its
    /// block or its `start` (`spec/27 §3`).
    task_depth: usize,
    /// The deadline of the iteration of the `for ever within` in progress (ADR 0116 rule 11).
    deadline: Option<Deadline>,
    /// The `within`s in progress, outermost first: each one's id and when its deadline passes
    /// on the run's clock, `None` beyond it (`spec/27 §5`).
    withins: Vec<(u64, Option<u64>)>,
    /// The `within`s an operation in progress shields, the first ones of `withins`: an
    /// operation completes whatever deadline passes meanwhile (`spec/19 §6` rule 4, `spec/26
    /// §7` rule 3), and only the `within`s its handler's own code starts can cut that code.
    within_shield: usize,
    /// The `tasks` statements whose cancellation cancels this task: the one that started it
    /// and those that started its ancestors (`spec/27 §7`); and how many of them an operation
    /// in progress shields, as `within_shield` does.
    cancels: Vec<Rc<TasksScope>>,
    cancel_shield: usize,
    /// The `tasks` statements this task runs, innermost last, whose tasks its `start`s start.
    scopes: Vec<Rc<TasksScope>>,
    /// The events of this task not yet written (ADR 0117 rule 10): an operation's until it
    /// ends, and every one while a branch's buffer is open.
    events: Vec<Event>,
    /// The branches in progress whose events wait for their block's join, innermost last.
    buffers: Vec<Buffer>,
    /// The operations of this task in progress.
    ops_open: usize,
    /// What the function in progress evaluates after the expression in progress, innermost
    /// last, with the names that each rebinds before reading them: a binding none of them
    /// reads is dead once the expression no longer reads it (#201).
    later: Vec<(&'p Expr, Vec<&'p str>)>,
    /// `later` of the functions that called the one in progress.
    callers: Vec<Vec<(&'p Expr, Vec<&'p str>)>>,
    /// The table of dense kernels (ADR 0120 rule 56), from the limits.
    kernels: &'static una_core::dense::variants::Kernels,
}

impl<'p, 'w> Machine<'p, 'w> {
    /// A machine over `index`, with `world`, `grants` and the keys of `keys:` grants for host
    /// capabilities, the limits of the run, and its clock: the root task of a new run.
    pub(crate) fn new(
        index: &'w Index<'p>,
        world: &'w mut dyn World,
        grants: &'w [Grant],
        keys: &'w BTreeMap<String, Vec<u8>>,
        limits: Limits<'w>,
        clock: Clock,
    ) -> Machine<'p, 'w> {
        let max_depth = limits.max_depth;
        let stop = limits.stop.map_or_else(
            || RunStop::new(crate::stopping::DEFAULT_GRACE_SECS),
            Arc::clone,
        );
        // The thread's attention word as it stands, read before the hard stop is: a hard stop
        // after the read raises the word, and one before it is seen here, with a value no word
        // holds, so the first check point takes the slow path.
        let word = una_sys::attention::register();
        let seen = word.load();
        let seen = if stop.hard().is_some() {
            u64::MAX
        } else {
            seen
        };
        let mut run = Run::new(world, clock, index.tasks);
        run.stop_after.set(limits.stop_after);
        // A run that serves live peers makes its `Http` requests off its thread when it has a
        // `tasks` statement (#1730); a script's engine keeps the reference schedule.
        // Its requests' sockets take the places of `spec/27 §2` rule 3, counted as native code
        // counts them.
        if limits.serve.is_some_and(|port| port.live_peers())
            && index.tasks
            && !run.clock.is_virtual()
        {
            let places = limits.http_places.unwrap_or_else(|| {
                una_http::serve::request_places(
                    una_sys::io::descriptor_limit(),
                    una_http::serve::reserved_descriptors(),
                )
            });
            run.offload = Some(crate::tasks::Offload::new(places));
        }
        run.serve = limits.serve.map(serve::Serving::new);
        // The run's limits refill on the run's clock (ADR 0116 rule 19).
        run.http_limits = Arc::new(una_http::limit::Governor::with_clock(
            grants
                .iter()
                .filter_map(|grant| match grant {
                    Grant::Http(http) => Some(http.clone()),
                    _ => None,
                })
                .collect(),
            Arc::new(run.clock.clone()),
        ));
        Machine {
            index,
            grants,
            keys,
            http: limits.http,
            run: Rc::new(run),
            depth: 0,
            max_depth,
            stop,
            in_ever: false,
            quiet: 0,
            hushed: 0,
            word,
            seen: Cell::new(seen),
            max_memory: limits.max_memory,
            held: limits.held,
            kernels: limits.kernels,
            baseline: (limits.held)(),
            stack_base: stack_position(&max_depth),
            stack_budget: crate::stack_bytes(max_depth)
                .map_or(usize::MAX, |bytes| bytes.saturating_sub(STACK_RESERVE)),
            frames: Vec::new(),
            task: Vec::new(),
            races: vec![crate::concurrency::BlockCounts::default()],
            tasks_runs: vec![crate::concurrency::BlockCounts::default()],
            task_depth: limits.task_depth,
            deadline: None,
            withins: Vec::new(),
            within_shield: 0,
            cancels: Vec::new(),
            cancel_shield: 0,
            scopes: Vec::new(),
            events: Vec::new(),
            buffers: Vec::new(),
            ops_open: 0,
            later: Vec::new(),
            callers: Vec::new(),
        }
    }

    /// The machine of the task `part` of the statement `scope`, `k` or, in quiet code, `q`
    /// (S-40), started here by this task: it shares the run, inherits the deadlines and
    /// cancellations in force, and may make `--task-depth` calls below the `start`, within the
    /// maximum in force (`spec/27 §7`, ADR 0117 amendment of 2026-10-19 on S3, point 3).
    fn child(&self, scope: &Rc<TasksScope>, part: String) -> Machine<'p, 'w> {
        let mut task = scope.path.clone();
        task.push(part);
        let mut cancels: Vec<Rc<TasksScope>> = self
            .cancels
            .iter()
            .skip(self.cancel_shield)
            .cloned()
            .collect();
        cancels.push(Rc::clone(scope));
        let stack_budget = crate::stack_bytes(self.task_depth)
            .map_or(usize::MAX, |bytes| bytes.saturating_sub(STACK_RESERVE));
        Machine {
            index: self.index,
            grants: self.grants,
            keys: self.keys,
            http: self.http,
            // A task runs the kernels of the run that started it (ADR 0120 rule 56).
            kernels: self.kernels,
            run: Rc::clone(&self.run),
            depth: self.depth,
            max_depth: self
                .max_depth
                .min(self.depth.saturating_add(self.task_depth)),
            stop: Arc::clone(&self.stop),
            in_ever: false,
            // A task started under a raised program counter runs under it (`spec/27 §7` rule 3).
            quiet: self.quiet,
            hushed: self.hushed,
            word: self.word,
            seen: Cell::new(self.seen.get()),
            max_memory: self.max_memory,
            held: self.held,
            baseline: self.baseline,
            // Set when the task's coroutine starts, on its own stack.
            stack_base: 0,
            stack_budget,
            frames: self.frames.last().copied().into_iter().collect(),
            task,
            races: vec![crate::concurrency::BlockCounts::default()],
            tasks_runs: vec![crate::concurrency::BlockCounts::default()],
            task_depth: self.task_depth,
            deadline: self.deadline.clone(),
            withins: self
                .withins
                .iter()
                .skip(self.within_shield)
                .copied()
                .collect(),
            within_shield: 0,
            cancels,
            cancel_shield: 0,
            scopes: Vec::new(),
            events: Vec::new(),
            buffers: Vec::new(),
            ops_open: 0,
            later: Vec::new(),
            callers: Vec::new(),
        }
    }

    /// Gives the run its process key of keyed hashing, read before any effect when its code
    /// can make a hash table (`spec/28 §1` rule 3), else `None`: every task of the run takes
    /// the keys of its tables from it.
    pub(crate) fn keyed(&self, hashing: Option<una_hash::ProcessKey>) {
        *self.run.hashing.borrow_mut() = hashing;
    }

    /// Calls `main` with no arguments and a host instance for each of its `uses`.
    pub(crate) fn run_main(&mut self, main: &'p Fun) -> Flow<'p> {
        let caps: BTreeMap<&'p str, Rc<Instance<'p>>> = main
            .uses
            .iter()
            .map(|u| {
                let id = format!("host:{}", u.capability);
                (u.cap.as_str(), Rc::new(Instance::Host { id }))
            })
            .collect();
        self.frames.push(main.span);
        let run = Rc::clone(&self.run);
        let _root = serve::RootRights(&run);
        let result = self.eval(&mut Env::new(), &Rc::new(caps), &main.body);
        let result = self.locate(result);
        self.frames.pop();
        // The root task's end answers the reply rights it still holds (ADR 0123 rule 4.5).
        self.answer_held(&result, false);
        result
    }

    /// Calls `fun`, a function the host calls, with `args` and the host capabilities of its
    /// `uses`, as `main` is run.
    fn call_hosted(&mut self, fun: &'p Fun, args: Vec<Value<'p>>) -> Flow<'p> {
        let caps: BTreeMap<&'p str, Rc<Instance<'p>>> = fun
            .uses
            .iter()
            .map(|u| {
                let id = format!("host:{}", u.capability);
                (u.cap.as_str(), Rc::new(Instance::Host { id }))
            })
            .collect();
        self.frames.push(fun.span);
        let mut env: Env<'p> = fun
            .params
            .iter()
            .map(|(name, _)| name.as_str())
            .zip(args)
            .collect();
        let result = self.eval(&mut env, &Rc::new(caps), &fun.body);
        let result = self.locate(result);
        self.frames.pop();
        result
    }

    /// Runs the host-driven step (ADR 0116 rule 3): `init` once, then `step` once per tick of
    /// `drive.ticks` milliseconds on the run's clock (real or virtual), delivering the latest due tick with the
    /// count of the due ticks skipped before it, then `on_stop` when one is declared. The loop
    /// stops at the start of an iteration once `drive.stop_after` of its own iterations have
    /// started (the budget of `--stop-after-iterations`, rule 19), or after a host stop. A failure of `init` ends the run as one of `main` does; `step` and `on_stop`
    /// cannot fail (U3029).
    pub(crate) fn drive(
        &mut self,
        init: &'p Fun,
        step: &'p Fun,
        on_stop: Option<&'p Fun>,
        drive: &Drive,
    ) -> Flow<'p> {
        let run = Rc::clone(&self.run);
        let _root = serve::RootRights(&run);
        let result = self.drive_steps(init, step, on_stop, drive);
        // The root task's end answers the reply rights it still holds (ADR 0123 rule 4.5).
        self.answer_held(&result, false);
        result
    }

    /// The steps of [`Machine::drive`].
    fn drive_steps(
        &mut self,
        init: &'p Fun,
        step: &'p Fun,
        on_stop: Option<&'p Fun>,
        drive: &Drive,
    ) -> Flow<'p> {
        let mut state = self.call_hosted(init, Vec::new())?;
        // Tick `k` is due `k * ticks` after `init` returns, on the run's clock: the monotonic
        // one, or the virtual one, which the wait moves at once (ADR 0116 rule 19).
        let origin = self.run.clock.now();
        let period = drive.ticks.max(1);
        let mut next: u64 = 0;
        // The iterations the driver's loop has started: its own budget (rule 19, #1472).
        let mut started: u64 = 0;
        let ended = loop {
            if self.ever_start(&mut started) {
                self.emit(Event {
                    seq: 0,
                    task: String::new(),
                    instance: String::new(),
                    capability: String::new(),
                    op: String::new(),
                    args: Vec::new(),
                    outcome: EventOutcome::Stop,
                });
                break Ok(());
            }
            // Tick `k` is due `k * ticks` after the first: wait for the next one, or deliver the
            // latest one due, with the count of those skipped. On the virtual clock the wait
            // moves the clock to the tick at once (rule 19); on the real one a host stop
            // interrupts it, and the next iteration start stops (rule 3).
            let due = origin.saturating_add(next.saturating_mul(period));
            if self.run.clock.is_virtual() {
                self.run.clock.wait_until(due);
            } else {
                let now = self.run.clock.now();
                if now < due
                    && !crate::stopping::wait_tick(
                        &self.stop,
                        std::time::Duration::from_millis(due - now),
                    )
                {
                    continue;
                }
            }
            let elapsed = self.run.clock.now().saturating_sub(origin);
            let latest = (elapsed / period).max(next);
            let missed = latest - next;
            let at = latest.saturating_mul(drive.ticks);
            next = latest + 1;
            let tick = Value::data(
                Ctor {
                    kind: Path {
                        module: "std.drive".to_owned(),
                        name: "Tick".to_owned(),
                    },
                    name: "Tick".to_owned(),
                    index: 0,
                },
                vec![
                    Value::int(i64::try_from(at).unwrap_or(i64::MAX)),
                    Value::int(i64::try_from(missed).unwrap_or(i64::MAX)),
                ],
            );
            // `step(edit state, input)` returns `(state', unit)` in Core (`spec/17 §3`). It is
            // the body of the host's `for ever`, so a host stop interrupts its `Http` waits.
            self.in_ever = true;
            let stepped = self.call_hosted(step, vec![state.clone(), tick]);
            self.in_ever = false;
            match stepped {
                Ok(Value::Tuple(parts)) => match parts.first() {
                    Some(updated) => state = updated.clone(),
                    None => {
                        break Err(Stop::Abort(Abort::Fault {
                            message: "`step` returned no state".to_owned(),
                            span: Some(step.span),
                        }));
                    }
                },
                Ok(_) => {
                    break Err(Stop::Abort(Abort::Fault {
                        message: "`step` returned no state".to_owned(),
                        span: Some(step.span),
                    }));
                }
                Err(stop) => break Err(stop),
            }
        };
        ended?;
        match on_stop {
            Some(on_stop) => {
                self.call_hosted(on_stop, vec![state])?;
                Ok(Value::Unit)
            }
            None => Ok(Value::Unit),
        }
    }

    /// Evaluates `expr` in `env` without capabilities: an example's body, whose `given` lines
    /// install its instances, or an expectation.
    pub(crate) fn eval_closed(&mut self, env: &mut Env<'p>, expr: &'p Expr) -> Flow<'p> {
        self.eval(env, &Rc::new(BTreeMap::new()), expr)
    }

    /// Gives a fault without a location the innermost function in progress.
    fn locate(&self, result: Flow<'p>) -> Flow<'p> {
        match result {
            Err(Stop::Abort(Abort::Fault {
                message,
                span: None,
            })) => Err(Stop::Abort(Abort::Fault {
                message,
                span: self.frames.last().copied(),
            })),
            other => other,
        }
    }

    /// Enters a call: past the maximum depth, or when the stack the evaluator was given runs
    /// short before it, the run aborts with U6003 rather than overflow the stack, and so it
    /// does when the heap the run holds is over its budget (ADR 0061).
    fn enter(&mut self, span: Span) -> Result<(), Stop<'p>> {
        // A call is a check point of the task's turn: after 1024 of them, the next ready task
        // runs (ADR 0117 rules 11 and 12), as native code counts its calls.
        self.run.tick();
        // The tick may have let another task run: when its abort ended the whole run, this
        // call ends with the run (`spec/27 §7`), before the depth check, so a call past the
        // limit does not end its task alone after the run's end.
        if self.run.coroutines
            && let Some(abort) = self.run.aborted()
        {
            return Err(Stop::Abort(abort));
        }
        let used = self.stack_base.abs_diff(stack_position(&span));
        if self.depth >= self.max_depth || used > self.stack_budget {
            return Err(Stop::Abort(Abort::Depth {
                span,
                limit: self.max_depth,
            }));
        }
        self.within_budget(span)?;
        self.depth += 1;
        // A function starts with an environment of its own: what its caller evaluates later
        // reads none of its bindings.
        self.callers.push(std::mem::take(&mut self.later));
        Ok(())
    }

    /// The run's memory budget, in MiB.
    pub(crate) fn max_memory(&self) -> usize {
        self.max_memory
    }

    /// Whether the run, holding `bytes` more, stays within its memory budget: the [`Room`] of
    /// its writes (`spec/11 §7`, #1223).
    pub(crate) fn fits(&self, bytes: usize) -> bool {
        let budget = self.max_memory.saturating_mul(1024 * 1024);
        (self.held)()
            .saturating_sub(self.baseline)
            .saturating_add(bytes)
            <= budget
    }

    /// The abort of a write at `span` that does not fit the run's memory budget (U6003,
    /// `spec/11 §7`, #1223), as a call over the budget aborts.
    pub(crate) fn over_budget(&self, span: Span) -> Stop<'p> {
        Stop::Abort(Abort::Memory {
            span,
            limit: self.max_memory,
        })
    }

    /// Whether the heap the run holds is within its budget (ADR 0061): checked at each call
    /// and at each iteration of a `for` loop, at `span`, else the run aborts with U6003.
    fn within_budget(&self, span: Span) -> Result<(), Stop<'p>> {
        // ADR 0116 rule 16: the hard stop cancels the run at its next check point, found on
        // the slow path when the thread's attention word changed (rule 18).
        if let Some(now) = self.word.changed_since(self.seen.get()) {
            self.seen.set(now);
            if let Some(hard) = self.stop.hard() {
                return Err(Stop::Abort(Abort::HostStop { span, hard }));
            }
        }
        let budget = self.max_memory.saturating_mul(1024 * 1024);
        if (self.held)().saturating_sub(self.baseline) > budget {
            return Err(Stop::Abort(Abort::Memory {
                span,
                limit: self.max_memory,
            }));
        }
        // Another task's abort that ends the whole run (`spec/27 §7`).
        if self.run.coroutines
            && let Some(abort) = self.run.aborted()
        {
            return Err(Stop::Abort(abort));
        }
        // A task whose statement was cancelled stops here (`spec/27 §7`).
        if self.cancelled() {
            return Err(Stop::Cut(CANCEL));
        }
        // `spec/27 §5`: past the deadline of a `within` in progress, its expression is
        // cancelled here; the outermost one that passed reports it (rule 5, Nesting).
        if let Some(id) = self.passed_within(self.within_shield) {
            return Err(Stop::Cut(id));
        }
        Ok(())
    }

    /// Whether a `tasks` statement whose cancellation reaches this task, past what an
    /// operation in progress shields, was cancelled. O(statements).
    fn cancelled(&self) -> bool {
        self.cancels
            .iter()
            .skip(self.cancel_shield)
            .any(|scope| scope.cancelled.get())
    }

    /// Holds `event` until it can be written: at once in a streaming task with no operation in
    /// progress and no branch's buffer open (ADR 0117 rule 10).
    fn emit(&mut self, event: Event) {
        self.events.push(event);
        self.flush();
    }

    /// Writes the events the task holds, once no operation of it is in progress and no
    /// branch's buffer is open, numbering their `seq` in the order they are written.
    fn flush(&mut self) {
        if self.ops_open > 0 || !self.buffers.is_empty() || self.events.is_empty() {
            return;
        }
        for event in std::mem::take(&mut self.events) {
            self.run.write(event);
        }
    }

    /// Writes every event the task holds: at its end, or the run's.
    pub(crate) fn finish_trace(&mut self) {
        self.ops_open = 0;
        self.buffers.clear();
        self.flush();
    }

    /// How many instances of the handler `name` the level this task writes to installed: the
    /// innermost open branch's, or the run's.
    fn installed(&self, name: &str) -> usize {
        match self.buffers.last() {
            Some(buffer) => buffer.installs.get(name).copied().unwrap_or(0),
            None => self.run.installs.borrow().get(name).copied().unwrap_or(0),
        }
    }

    /// Counts `count` more instances of `name` at the level this task writes to.
    fn add_installs(&mut self, name: &str, count: usize) {
        match self.buffers.last_mut() {
            Some(buffer) => *buffer.installs.entry(name.to_owned()).or_insert(0) += count,
            None => {
                *self
                    .run
                    .installs
                    .borrow_mut()
                    .entry(name.to_owned())
                    .or_insert(0) += count
            }
        }
    }

    /// The id of the `count`-th instance of `name` at the level this task writes to: in a
    /// branch whose events wait for its join, `with:<name>#@<path>#<count>`, which the join
    /// renumbers; elsewhere `with:<name>#<count>`, counted run-wide (`spec/19 §7` rule 2,
    /// `spec/27 §7`).
    fn renamed(&self, name: &str, count: usize) -> String {
        match self.buffers.last() {
            Some(buffer) => format!("with:{name}#@{}#{count}", buffer.path),
            None => format!("with:{name}#{count}"),
        }
    }

    /// The id of a new instance of the handler `name` installed by `with`. Quiet code counts
    /// none of its installations: their ids read `with:<name>#q`, so whether and how often it
    /// ran shows in no id (S-41, `spec/26 §6` rule 4). O(log h) for h installed handler names.
    fn install_id(&mut self, name: &str) -> String {
        if self.quiet > 0 {
            return crate::concurrency::quiet_install_id(name);
        }
        let count = self.installed(name) + 1;
        self.add_installs(name, 1);
        self.renamed(name, count)
    }

    /// The join of a block whose branches ended as `parts`, given in the order the join takes
    /// them: their events, held from `start` on, follow in that order, and the ids of the
    /// handlers each one installed are renumbered after those installed before, as native
    /// code's join adopts its tasks' events (`spec/19 §7` rule 2, `spec/27 §6`). Then the task
    /// writes what it can. O(events + installations of the block).
    fn adopt(&mut self, start: usize, parts: &[&Part]) {
        let mut slots: Vec<Option<Event>> = self
            .events
            .split_off(start.min(self.events.len()))
            .into_iter()
            .map(Some)
            .collect();
        for part in parts {
            let local = format!("#@{}#", part.buffer.path);
            let bases: BTreeMap<&str, usize> = part
                .buffer
                .installs
                .keys()
                .map(|name| (name.as_str(), self.installed(name)))
                .collect();
            let range = part.buffer.start.saturating_sub(start)..part.end.saturating_sub(start);
            for slot in slots.get_mut(range).into_iter().flatten() {
                let Some(mut event) = slot.take() else {
                    continue;
                };
                if let Some(rest) = event.instance.strip_prefix("with:")
                    && let Some((name, count)) = rest.split_once(&local)
                    && let Ok(count) = count.parse::<usize>()
                    && let Some(&base) = bases.get(name)
                {
                    event.instance = self.renamed(name, base + count);
                }
                self.events.push(event);
            }
            for (name, &count) in &part.buffer.installs {
                self.add_installs(name, count);
            }
        }
        // Branches hold contiguous events, so none is left; any would keep its place.
        self.events.extend(slots.into_iter().flatten());
        self.flush();
    }

    /// Runs `f` on this machine, the root task of its run: as a coroutine that takes turns
    /// with the tasks it starts, when the program has a `tasks` statement, under the driver of
    /// the run's queue (ADR 0117 rule 11); on the thread's own stack otherwise, at no cost.
    /// Writes the events the root holds when `f` returns.
    ///
    /// The abort when the root cannot run as a coroutine: its stack cannot be mapped (U6003),
    /// or the interpreter already runs on a coroutine, which none of its callers does (a
    /// fault). A panic of the interpreter in a task is caught on that task's stack and raised
    /// again here, on the thread's, once every task has ended, so the command reports it.
    pub(crate) fn rooted<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> Result<R, Abort> {
        if !self.run.coroutines {
            let result = f(self);
            self.finish_trace();
            return Ok(result);
        }
        if una_sys::coroutine::inside() {
            return Err(Abort::Fault {
                message: "the interpreter runs its tasks on a coroutine of another owner"
                    .to_owned(),
                span: None,
            });
        }
        let stack = crate::stack_bytes(self.max_depth)
            .and_then(|bytes| una_sys::coroutine::Stack::new(bytes).ok());
        let Some(stack) = stack else {
            return Err(Abort::Unstarted {
                span: None,
                cap: None,
            });
        };
        let run = Rc::clone(&self.run);
        let mut out = None;
        {
            let slot = &mut out;
            let machine = &mut *self;
            let guard = Rc::clone(&run);
            una_sys::coroutine::scope(|scope| {
                let root = scope.spawn(stack, move || {
                    guard.guarded(|| {
                        let base = 0usize;
                        let saved =
                            std::mem::replace(&mut machine.stack_base, stack_position(&base));
                        *slot = Some(f(machine));
                        machine.stack_base = saved;
                        machine.finish_trace();
                    });
                });
                crate::tasks::drive(scope, &run, root);
            });
        }
        if let Some(payload) = run.take_panic() {
            std::panic::resume_unwind(payload);
        }
        out.ok_or_else(|| Abort::Fault {
            message: "the root task did not run to its end".to_owned(),
            span: None,
        })
    }

    /// The outermost `within` in progress from position `from` on whose deadline has passed on
    /// the run's clock. O(withins), and no clock reading without one.
    fn passed_within(&self, from: usize) -> Option<u64> {
        if self.withins.len() <= from {
            return None;
        }
        let now = self.run.clock.now();
        self.withins
            .iter()
            .skip(from)
            .find(|(_, at)| at.is_some_and(|at| at <= now))
            .map(|(id, _)| *id)
    }

    /// The time left before the earliest deadline of the `within`s in progress, zero once it
    /// has passed; `None` without one that passes.
    fn within_left(&self) -> Option<std::time::Duration> {
        let at = self.withins.iter().filter_map(|(_, at)| *at).min()?;
        Some(std::time::Duration::from_millis(
            at.saturating_sub(self.run.clock.now()),
        ))
    }

    /// `e within d` (`spec/27 §5`, ADR 0117 rule 5): `body` bounded by the deadline `ms`
    /// milliseconds after now. Its outcome stands when it returns before the deadline, which is
    /// checked once more when it returns; past it, the `within` fails with `timeout` after its
    /// `timeout` event. A failure `x` in time fails as `wrap(x)`, or as `x` without a `wrap`.
    /// An outer `within` whose deadline passed too reports instead.
    fn within(
        &mut self,
        env: &mut Env<'p>,
        caps: &Caps<'p>,
        ms: u64,
        body: &'p Expr,
        timeout: &'p Ctor,
        wrap: Option<&'p Ctor>,
    ) -> Flow<'p> {
        let id = self.run.next_within.get();
        self.run.next_within.set(id + 1);
        let at = self.run.clock.now().checked_add(ms);
        let depth = self.withins.len();
        self.withins.push((id, at));
        let result = self.eval(env, caps, body);
        self.withins.truncate(depth);
        let timed_out = |machine: &mut Self| {
            machine.within_timeout_event();
            Err(Stop::Fail(Value::data(timeout.clone(), Vec::new())))
        };
        match result {
            Err(Stop::Cut(cut)) if cut == id => timed_out(self),
            Err(stop @ (Stop::Cut(_) | Stop::Abort(_) | Stop::Exit(..))) => Err(stop),
            result => {
                if let Some(outer) = self.passed_within(self.within_shield) {
                    return Err(Stop::Cut(outer));
                }
                if at.is_some_and(|at| at <= self.run.clock.now()) {
                    return timed_out(self);
                }
                match (result, wrap) {
                    (Err(Stop::Fail(failure)), Some(wrap)) => {
                        Err(Stop::Fail(Value::data(wrap.clone(), vec![failure])))
                    }
                    (result, _) => result,
                }
            }
        }
    }

    /// The `cancelled` events of the tasks `from..to` of the block in progress, which a cut
    /// stopped or kept from starting (`spec/27 §5`), in task order: each task the cut stops ends
    /// with one, as natively, whatever had started. None inside a `quiet` expression (S-39,
    /// `spec/26 §6` rule 4), whose program counter they would reveal.
    fn cut_events(&mut self, from: usize, to: usize) {
        if self.quiet > 0 {
            return;
        }
        for index in from..to {
            let mut path: Vec<String> = self.task.iter().map(ToString::to_string).collect();
            path.push((index + 1).to_string());
            self.emit(Event {
                seq: 0,
                task: path.join("."),
                instance: String::new(),
                capability: String::new(),
                op: String::new(),
                args: Vec::new(),
                outcome: EventOutcome::Cancelled,
            });
        }
    }

    /// The `cancelled` events of the branches `from..to` of the `first of` named `race` (`f<c>`)
    /// of the task in progress, which a cut stopped or kept from starting (`spec/27 §5`, `§6`),
    /// in index order, as for the tasks of a block ([`Self::cut_events`]).
    fn race_cut_events(&mut self, race: &str, from: usize, to: usize) {
        if self.quiet > 0 {
            return;
        }
        for index in from..to {
            let mut path: Vec<String> = self.task.iter().map(ToString::to_string).collect();
            path.push(race.to_owned());
            path.push((index + 1).to_string());
            self.emit(Event {
                seq: 0,
                task: path.join("."),
                instance: String::new(),
                capability: String::new(),
                op: String::new(),
                args: Vec::new(),
                outcome: EventOutcome::Cancelled,
            });
        }
    }

    /// Appends the `timeout` event of a `within` of the task in progress (`spec/27 §5`); none
    /// inside a `quiet` expression (S-39, `spec/26 §6` rule 4).
    fn within_timeout_event(&mut self) {
        if self.quiet > 0 {
            return;
        }
        let task = self.task.join(".");
        self.emit(Event {
            seq: 0,
            task,
            instance: String::new(),
            capability: String::new(),
            op: String::new(),
            args: Vec::new(),
            outcome: EventOutcome::Timeout,
        });
    }

    /// Structural equality at `span`, within the memory budget: a comparison visits at most as
    /// many pairs of composite values as the budget holds at `EQUALITY_PAIR_BYTES` each, else the
    /// run aborts with U6003 there (ADR 0061, amendment of 2026-10-17, #815). The count is fixed
    /// by the values, not by what the heap holds, so the native backend stops at the same pair.
    fn equal_at(&self, a: &Value<'p>, b: &Value<'p>, span: Span) -> Result<bool, Stop<'p>> {
        let budget = self.max_memory.saturating_mul(1024 * 1024);
        equal_within(a, b, budget / EQUALITY_PAIR_BYTES).ok_or(Stop::Abort(Abort::Memory {
            span,
            limit: self.max_memory,
        }))
    }

    fn leave(&mut self, result: Flow<'p>) -> Flow<'p> {
        self.depth = self.depth.saturating_sub(1);
        self.later = self.callers.pop().unwrap_or_default();
        result
    }

    /// The names `expr` reads.
    fn reads(&mut self, expr: &'p Expr) -> Rc<BTreeSet<String>> {
        Rc::clone(
            self.run
                .reads
                .borrow_mut()
                .entry(std::ptr::from_ref(expr))
                .or_insert_with(|| {
                    let mut names = BTreeSet::new();
                    una_core::vars(expr, &mut names);
                    Rc::new(names)
                }),
        )
    }

    /// The names `expr` binds.
    fn binders(&mut self, expr: &'p Expr) -> Rc<BTreeSet<&'p str>> {
        Rc::clone(
            self.run
                .binders
                .borrow_mut()
                .entry(std::ptr::from_ref(expr))
                .or_insert_with(|| {
                    let mut names = BTreeSet::new();
                    una_core::binders(expr, &mut names);
                    Rc::new(names)
                }),
        )
    }

    /// Whether the binding `name` is dead once `rest` has started: neither `rest` nor what the
    /// function evaluates after it reads it (#201).
    fn dead_after(&mut self, name: &str, rest: &'p Expr) -> bool {
        if self.reads(rest).contains(name) {
            return false;
        }
        // Taken out and put back rather than cloned: each update asks this.
        let later = std::mem::take(&mut self.later);
        let dead = later
            .iter()
            .all(|(expr, rebound)| rebound.contains(&name) || !self.reads(expr).contains(name));
        self.later = later;
        dead
    }

    /// The bindings of `init`, the values a loop's state starts from, that nothing reads once
    /// the loop has started, neither its source, its condition, its body, its `on stop` block
    /// nor what follows it: they give their values up to the state, so that the body, holding
    /// the only reference, updates a value in place, as a call's argument at its last use does
    /// (#30, #201). Without this, a nested loop over an `edit` binding copies the value at each
    /// entry of the inner loop, O(n) per entry. A value something else holds is still copied
    /// when written.
    fn give_up_init(
        &mut self,
        env: &mut Env<'p>,
        source: &'p LoopSource,
        init: &'p [Atom],
        body: &'p Expr,
        on_stop: Option<&'p una_core::OnStop>,
    ) {
        let mut read: Vec<&'p str> = Vec::new();
        let mut var = |atom: &'p Atom| {
            if let Atom::Var(name) = atom {
                read.push(name.as_str());
            }
        };
        let cond = match source {
            LoopSource::List(list) => {
                var(list);
                None
            }
            LoopSource::Range { low, high, .. } => {
                var(low);
                var(high);
                None
            }
            LoopSource::Until { cond, .. } => Some(self.reads(cond)),
            LoopSource::Ever { within } => {
                if let Some((deadline, _)) = within {
                    var(deadline);
                }
                None
            }
        };
        let block = on_stop.map(|on_stop| self.reads(&on_stop.body));
        for atom in init {
            let Atom::Var(name) = atom else {
                continue;
            };
            let kept = read.contains(&name.as_str())
                || cond.as_ref().is_some_and(|names| names.contains(name))
                || block.as_ref().is_some_and(|names| names.contains(name));
            if !kept
                && self.dead_after(name, body)
                && let Some(value) = env.lookup_mut(name)
            {
                *value = Value::Unit;
            }
        }
    }

    /// `let t2 = concat(t1, part, ...) in rest`, the interpolation `"{t}..."`, where the text of
    /// `t1` is held by no binding that `rest` or what follows reads, as when `t = "{t}x"` in a
    /// loop: those bindings give the text up, and `Rc::make_mut` appends to it in place when
    /// that leaves its only reference, or copies it otherwise (ADR 0070 X10, `spec/22 §6` rule
    /// 2, #261). `None` when the rhs is not such a concatenation.
    fn appends_in_place(
        &mut self,
        env: &mut Env<'p>,
        rhs: &'p Rhs,
        rest: &'p Expr,
    ) -> Option<Flow<'p>> {
        let Rhs::Prim {
            prim: Prim::Concat,
            args,
            ..
        } = rhs
        else {
            return None;
        };
        let Some(Atom::Var(first)) = args.first() else {
            return None;
        };
        let parts = match self.atoms(env, args) {
            Ok(parts) => parts,
            Err(stop) => return Some(Err(stop)),
        };
        let mut parts = parts.into_iter();
        let Some(Value::Text(mut text)) = parts.next() else {
            return None;
        };
        if !self.dead_after(first, rest) {
            return None;
        }
        // Every binding of this text that nothing reads later gives it up: in `"{t}x"`, both
        // `t` and the temporary of its conversion to text.
        let dead: Vec<usize> = env
            .iter()
            .enumerate()
            .filter(
                |(_, (_, value))| matches!(value, Value::Text(other) if Rc::ptr_eq(other, &text)),
            )
            .map(|(at, _)| at)
            .collect();
        for at in dead {
            let name = env.get(at).map(|(name, _)| *name)?;
            if self.dead_after(name, rest)
                && let Some(value) = env.get_mut(at)
            {
                *value = Value::Unit;
            }
        }
        let grown = Rc::make_mut(&mut text);
        for part in parts {
            match part {
                Value::Text(part) => grown.push_str(&part),
                _ => return Some(Err(fault("`concat` of a value that is not a `Text`"))),
            }
        }
        Some(Ok(Value::Text(text)))
    }

    /// `let name = f(x, ...) in rest` where arguments of the call are bindings at their last
    /// use: those bindings give up their values to the callee, which, holding the only
    /// reference, updates them in place, as the native code does (#30, #201; ADR 0072, #262).
    /// `rest` includes the code after a failure of the call (a handler, or what follows an
    /// `attempt`, through `later`), so a binding read there keeps its value. A binding passed
    /// twice keeps it too, and a value something else holds is still copied, so no value ever
    /// changes under another name.
    /// A map's `set` and `remove`, `With` and `Without`, give up the map the same way
    /// (`spec/21 §5` rule 2), and so does a call through a function value.
    fn updates_in_place(&mut self, rhs: &'p Rhs, rest: &'p Expr) -> Option<Vec<&'p str>> {
        let args = match rhs {
            // A call through a function value too, whose lambda owns its arguments.
            Rhs::Call { args, .. } | Rhs::Apply { args, .. } => args.as_slice(),
            Rhs::Prim {
                prim: Prim::Map(MapOp::With | MapOp::Without),
                args,
                ..
            } => args.get(..1)?,
            // A `Stack` or a `Queue` given up to a push or a removal (`spec/28 §5`).
            Rhs::Prim {
                prim: Prim::Seq(op),
                args,
                ..
            } if op.rebuilds() => args.get(..1)?,
            // A set given up to `add`, `remove`, `with`, `without` or a set operation that
            // builds on its receiver (`spec/28 §6`).
            Rhs::Prim {
                prim: Prim::Set(op),
                args,
                ..
            } if op.rebuilds() => args.get(..1)?,
            // A hash map or a hash set given up the same way (`spec/28 §7`).
            Rhs::Prim {
                prim: Prim::HashMap(op),
                args,
                ..
            } if op.rebuilds() => args.get(..1)?,
            Rhs::Prim {
                prim: Prim::HashSet(op),
                args,
                ..
            } if op.rebuilds() => args.get(..1)?,
            // A priority queue given up to a push or a removal (`spec/28 §8`).
            Rhs::Prim {
                prim: Prim::Priority(op),
                args,
                ..
            } if op.rebuilds() => args.get(..1)?,
            // A bit set given up to `add`, `remove`, `with`, `without` or a set operation that
            // builds on its receiver (`spec/28 §9`).
            Rhs::Prim {
                prim: Prim::BitSet(op),
                args,
                ..
            } if op.rebuilds() => args.get(..1)?,
            _ => return None,
        };
        let all = match rhs {
            Rhs::Call { args, .. } | Rhs::Apply { args, .. } | Rhs::Prim { args, .. } => {
                args.as_slice()
            }
            _ => args,
        };
        let mut moved = Vec::new();
        for arg in args {
            let Atom::Var(name) = arg else {
                continue;
            };
            let twice = all
                .iter()
                .filter(|arg| matches!(arg, Atom::Var(other) if other == name))
                .count()
                > 1;
            if !twice && self.dead_after(name, rest) {
                moved.push(name.as_str());
            }
        }
        (!moved.is_empty()).then_some(moved)
    }

    fn eval(&mut self, env: &mut Env<'p>, caps: &Caps<'p>, expr: &'p Expr) -> Flow<'p> {
        let mut expr = expr;
        let mut caps = caps.clone();
        loop {
            match expr {
                Expr::Atom { atom, .. } => return self.atom(env, atom),
                // The `exhaustive` obligation proves this branch is never
                // taken; reaching it is a fault.
                Expr::Unreachable { goal, .. } => {
                    return Err(fault(format!(
                        "no arm of a `when` matches, which the obligation `{goal}` excludes"
                    )));
                }
                Expr::Let {
                    name,
                    ty,
                    value,
                    body,
                    ..
                } => {
                    let value = match value.as_ref() {
                        Bound::Rhs(rhs) => match self.updates_in_place(rhs, body) {
                            Some(moved) => {
                                let (Rhs::Call { args, .. }
                                | Rhs::Apply { args, .. }
                                | Rhs::Prim { args, .. }) = rhs
                                else {
                                    return Err(fault(
                                        "an update that is neither a call nor a map's",
                                    ));
                                };
                                let callee = match rhs {
                                    Rhs::Apply { callee, .. } => Some(self.atom(env, callee)?),
                                    _ => None,
                                };
                                let args = self.atoms(env, args)?;
                                for moved in moved {
                                    if let Some(value) = env.lookup_mut(moved) {
                                        *value = Value::Unit;
                                    }
                                }
                                match rhs {
                                    Rhs::Call { function, span, .. } => {
                                        self.call(function, args, &caps, *span)?
                                    }
                                    Rhs::Apply { span, .. } => match &callee {
                                        Some(callee) => self.apply(callee, args, *span)?,
                                        None => return Err(fault("a call of no function value")),
                                    },
                                    Rhs::Prim {
                                        prim: Prim::Map(op),
                                        ..
                                    } => map_apply(*op, args)?,
                                    Rhs::Prim {
                                        prim: Prim::Seq(op),
                                        ..
                                    } => seqs::seq_apply(*op, args)?,
                                    Rhs::Prim {
                                        prim: Prim::Set(op),
                                        ..
                                    } => sets::set_apply(*op, args)?,
                                    Rhs::Prim {
                                        prim: prim @ (Prim::HashMap(_) | Prim::HashSet(_)),
                                        span,
                                        ..
                                    } => self.hash_apply(*prim, args, *span)?,
                                    Rhs::Prim {
                                        prim: Prim::Priority(op),
                                        ..
                                    } => priority::priority_apply(*op, args)?,
                                    Rhs::Prim {
                                        prim: Prim::BitSet(op),
                                        span,
                                        ..
                                    } => bitsets::bit_set_apply(*op, args, Some(*span))?,
                                    _ => return Err(fault("an update that is not one")),
                                }
                            }
                            None => match self.appends_in_place(env, rhs, body) {
                                Some(appended) => appended?,
                                None => self.rhs(env, &caps, rhs, ty)?,
                            },
                        },
                        Bound::Expr(inner) => {
                            self.later.push((body, Vec::new()));
                            let value = self.eval(env, &caps, inner);
                            self.later.pop();
                            value?
                        }
                    };
                    env.push((name.as_str(), value));
                    expr = body;
                }
                // A `for` loop (`spec/17 §4`): iterations are not calls, so they do not count
                // toward the call depth (ADR 0056, amendment of 2026-10-02).
                Expr::Loop {
                    source,
                    var,
                    state,
                    init,
                    body,
                    on_stop,
                    span,
                } => {
                    let mut current = self.atoms(env, init)?;
                    self.give_up_init(env, source, init, body, on_stop.as_deref());
                    // `on stop` runs after a failure of the body with the state from before
                    // the loop (ADR 0116 rule 10).
                    let before = on_stop.as_ref().map(|_| current.clone());
                    // The next iterations read the bindings the body reads, but for the item,
                    // the state and the binders of the body, which each iteration binds anew
                    // before reading them: Core binders are unique (#261).
                    let until = match source.as_ref() {
                        LoopSource::Until { cond, .. } => Some(cond.as_ref()),
                        _ => None,
                    };
                    let in_cond = until.map(|cond| self.binders(cond));
                    let rebound = std::iter::once(var.as_ref())
                        .chain(state.iter())
                        .map(|(name, _)| name.as_str())
                        .chain(self.binders(body).iter().copied())
                        .chain(in_cond.iter().flat_map(|names| names.iter().copied()))
                        .collect::<Vec<_>>();
                    // The condition of `for until` is evaluated again before each iteration,
                    // so it is later too, for the body and for the condition itself (#1202).
                    let mark = self.later.len();
                    if let Some(cond) = until {
                        self.later.push((cond, rebound.clone()));
                    }
                    self.later.push((body, rebound));
                    let result = self.iterations(
                        env,
                        &caps,
                        (
                            source.as_ref(),
                            var.as_ref(),
                            state.as_slice(),
                            body.as_ref(),
                            *span,
                        ),
                        current,
                    );
                    self.later.truncate(mark);
                    let Some(on_stop) = on_stop else {
                        current = result?;
                        return Ok(Value::Tuple(Rc::new(current)));
                    };
                    return match result {
                        Ok(last) => {
                            let last = self.on_stop(env, &caps, on_stop, last)?;
                            Ok(Value::Tuple(Rc::new(last)))
                        }
                        Err(Stop::Fail(failure)) => {
                            self.on_stop(env, &caps, on_stop, before.unwrap_or_default())?;
                            Err(Stop::Fail(failure))
                        }
                        // A `within` around the loop passed its deadline: the loop ends as after
                        // a failure, and its `on stop` runs, shielded from that deadline, before
                        // the cut goes on (ADR 0116 rule 15, ADR 0117 rule 5).
                        Err(Stop::Cut(id)) => {
                            let shield =
                                std::mem::replace(&mut self.within_shield, self.withins.len());
                            let cancel_shield =
                                std::mem::replace(&mut self.cancel_shield, self.cancels.len());
                            let ran = self.on_stop(env, &caps, on_stop, before.unwrap_or_default());
                            self.cancel_shield = cancel_shield;
                            self.within_shield = shield;
                            ran?;
                            Err(Stop::Cut(id))
                        }
                        Err(other) => Err(other),
                    };
                }
                // The tasks of a `together` block (`spec/19 §6`), run one after another in
                // branch order, the sequential schedule: a failure stops the block before
                // the tasks after it start, and none is cancelled after starting.
                Expr::Together { tasks, .. } => {
                    // A task's captures are never given up: the tasks after it read them. The
                    // bindings of the tasks are their own, bound anew in each before it reads
                    // them: Core binders are unique (#263).
                    let rebound = self.binders(expr).iter().copied().collect();
                    self.later.push((expr, rebound));
                    let mut results = Vec::with_capacity(tasks.len());
                    // The tasks' events wait for the join, which adopts them in task order.
                    let start = self.events.len();
                    let mut parts = Vec::with_capacity(tasks.len());
                    for (index, task) in tasks.iter().enumerate() {
                        let result = match self.passed_within(self.within_shield) {
                            Some(id) => Err(Stop::Cut(id)),
                            None => {
                                let (result, part) = self.task(env, &caps, index, task);
                                parts.push(part);
                                result
                            }
                        };
                        match result {
                            Ok(result) => results.push(result),
                            Err(stop) => {
                                self.later.pop();
                                self.adopt(start, &parts.iter().collect::<Vec<_>>());
                                if matches!(stop, Stop::Fail(_)) {
                                    self.joined();
                                }
                                if matches!(stop, Stop::Cut(_)) {
                                    self.cut_events(index, tasks.len());
                                }
                                return Err(stop);
                            }
                        }
                    }
                    self.later.pop();
                    self.adopt(start, &parts.iter().collect::<Vec<_>>());
                    self.joined();
                    return Ok(Value::Tuple(Rc::new(results)));
                }
                // `first of` (`spec/27 §6`): the branches in index order, up to the first that
                // returns. As for `together`, a branch's captures are never given up.
                Expr::FirstOf { tasks, .. } => {
                    let rebound = self.binders(expr).iter().copied().collect();
                    self.later.push((expr, rebound));
                    let result = self.first_of(env, &caps, tasks);
                    self.later.pop();
                    if matches!(result, Ok(_) | Err(Stop::Fail(_))) {
                        self.joined();
                    }
                    return result;
                }
                // `together for` (`spec/19 §6`): one task per element, in element order.
                Expr::TogetherFor {
                    source,
                    var,
                    task,
                    span,
                } => {
                    let items = self.elements(env, source, *span)?;
                    let mut results = Vec::with_capacity(items.len());
                    // As for `together`, and the element too, which each task binds anew (#263).
                    let rebound = self.binders(expr).iter().copied().collect();
                    self.later.push((expr, rebound));
                    let start = self.events.len();
                    let mut parts = Vec::with_capacity(items.len());
                    let count = items.len();
                    for (index, item) in items.into_iter().enumerate() {
                        let mark = env.len();
                        env.push((var.0.as_str(), item));
                        let result = match self.passed_within(self.within_shield) {
                            Some(id) => Err(Stop::Cut(id)),
                            None => {
                                let (result, part) = self.task(env, &caps, index, task);
                                parts.push(part);
                                result
                            }
                        };
                        env.truncate(mark);
                        match result {
                            Ok(result) => results.push(result),
                            Err(stop) => {
                                self.later.pop();
                                self.adopt(start, &parts.iter().collect::<Vec<_>>());
                                if matches!(stop, Stop::Fail(_)) {
                                    self.joined();
                                }
                                if matches!(stop, Stop::Cut(_)) {
                                    self.cut_events(index, count);
                                }
                                return Err(stop);
                            }
                        }
                    }
                    self.later.pop();
                    self.adopt(start, &parts.iter().collect::<Vec<_>>());
                    self.joined();
                    return Ok(Value::List(Rc::new(results)));
                }
                // `tasks(b) e` (`spec/27 §7`): the block in this task, then the wait for its
                // tasks.
                Expr::Tasks { bound, body, .. } => {
                    let bound = match bound {
                        Some((atom, _)) => match self.atom(env, atom)? {
                            // A bound below 1 is refuted by verification; one beyond `usize`
                            // bounds nothing.
                            Value::Int(n) => Some(if n.sign() == Sign::Minus {
                                1
                            } else {
                                usize::try_from(&n).unwrap_or(usize::MAX).max(1)
                            }),
                            _ => return Err(fault("a bound of `tasks` that is not an `Int`")),
                        },
                        None => None,
                    };
                    return self.tasks(env, &caps, bound, body);
                }
                // `start e` (`spec/27 §7`): a task for `e`, and the block goes on.
                Expr::Start { task, rights, span } => {
                    self.start(env, &caps, (task, rights), *span)?;
                    return Ok(Value::Unit);
                }
                // `quiet e` (S-39, `spec/26 §6` rule 4): `e`, then the count as before, on every
                // way out of it.
                Expr::Quiet { body, timing, .. } => {
                    let hush = u32::from(!*timing);
                    self.quiet = self.quiet.saturating_add(1);
                    self.hushed = self.hushed.saturating_add(hush);
                    let result = self.eval(env, &caps, body);
                    self.hushed = self.hushed.saturating_sub(hush);
                    self.quiet = self.quiet.saturating_sub(1);
                    return result;
                }
                // `let (x, ...) = a in e` (`spec/17 §4`).
                Expr::LetTuple {
                    names, value, body, ..
                } => {
                    let Value::Tuple(items) = self.atom(env, value)? else {
                        return Err(fault("`let (...)` of a value that is not a tuple"));
                    };
                    if items.len() != names.len() {
                        return Err(fault("`let (...)` of a tuple of another length"));
                    }
                    // A tuple binding at its last use, as the result of an update or of a
                    // loop is, gives its items up: the names bound to them then hold them
                    // once, and the next update of one happens in place (#30, #201).
                    if let Atom::Var(tuple) = value
                        && self.dead_after(tuple, body)
                        && let Some(bound) = env.lookup_mut(tuple)
                    {
                        *bound = Value::Unit;
                    }
                    let items = Rc::try_unwrap(items).unwrap_or_else(|shared| (*shared).clone());
                    for ((name, _), item) in names.iter().zip(items) {
                        env.push((name.as_str(), item));
                    }
                    expr = body;
                }
                Expr::If {
                    cond,
                    then,
                    otherwise,
                } => {
                    expr = match self.atom(env, cond)? {
                        Value::Bool(true) => then,
                        Value::Bool(false) => otherwise,
                        _ => return Err(fault("`if` on a value that is not a `Bool`")),
                    };
                }
                Expr::Case {
                    scrutinee,
                    arms,
                    otherwise,
                } => {
                    let Value::Data(data) = self.atom(env, scrutinee)? else {
                        return Err(fault("`case` on a value that is not a variant"));
                    };
                    let arm = arms.iter().find(|arm| {
                        arm.ctor.kind == data.ctor.kind && arm.ctor.index == data.ctor.index
                    });
                    match (arm, otherwise) {
                        (Some(arm), _) => {
                            for (bind, field) in arm.binds.iter().zip(&data.fields) {
                                if let Some(name) = bind {
                                    env.push((name.as_str(), field.clone()));
                                }
                            }
                            expr = &arm.body;
                        }
                        (None, Some(otherwise)) => expr = otherwise,
                        (None, None) => return Err(fault("no arm of `case` matches")),
                    }
                }
                Expr::Fail { ctor, args, .. } => {
                    let fields = self.atoms(env, args)?;
                    return Err(Stop::Fail(Value::data(ctor.clone(), fields)));
                }
                Expr::Exit { tag, state, .. } => {
                    let values = self.atoms(env, state)?;
                    return Err(Stop::Exit(*tag, values));
                }
                Expr::Handle {
                    body,
                    error,
                    handler,
                } => {
                    // After a failure of the body, the handler runs.
                    self.later.push((handler, Vec::new()));
                    let result = self.eval(env, &caps, body);
                    self.later.pop();
                    match result {
                        Err(Stop::Fail(failure)) => {
                            env.push((error.as_str(), failure));
                            expr = handler;
                        }
                        other => return other,
                    }
                }
                Expr::Attempt { body, .. } => {
                    return match self.eval(env, &caps, body) {
                        Ok(value) => Ok(Value::data(prelude_ctor("Result", "Ok", 0), vec![value])),
                        Err(Stop::Fail(failure)) => {
                            Ok(Value::data(prelude_ctor("Result", "Err", 1), vec![failure]))
                        }
                        Err(abort) => Err(abort),
                    };
                }
                Expr::Within {
                    deadline,
                    body,
                    timeout,
                    wrap,
                    ..
                } => {
                    // A deadline below 1 is refuted by verification, and has passed at once; one
                    // that does not fit `u64` never passes.
                    let ms = match self.atom(env, deadline)? {
                        Value::Int(n) => {
                            if n.sign() == Sign::Minus {
                                0
                            } else {
                                u64::try_from(&n).unwrap_or(u64::MAX)
                            }
                        }
                        _ => return Err(fault("a deadline that is not an `Int`")),
                    };
                    return self.within(env, &caps, ms, body, timeout, wrap.as_deref());
                }
                Expr::With { cap, source, body } => {
                    let instance = self.instance(env, &caps, cap, source)?;
                    let mut inner = (*caps).clone();
                    inner.insert(cap.as_str(), instance);
                    caps = Rc::new(inner);
                    expr = body;
                }
            }
        }
    }

    /// E-Rebind shares the instance; E-Handler builds a fresh one over the handler's own
    /// capabilities.
    fn instance(
        &mut self,
        env: &Env<'p>,
        caps: &Caps<'p>,
        cap: &str,
        source: &'p Source,
    ) -> Result<Rc<Instance<'p>>, Stop<'p>> {
        match source {
            Source::Rebind(other) => caps
                .get(other.as_str())
                .cloned()
                .ok_or_else(|| fault(format!("no capability `@{other}` to rebind"))),
            Source::Handler {
                handler,
                args,
                given,
                ..
            } => {
                let values = self.atoms(env, args)?;
                let id = if *given {
                    format!("given:{cap}")
                } else {
                    self.install_id(&handler.name)
                };
                // `limiter` of `std.limit` (ADR 0117 rule 9): the engine provides it, a bucket
                // over the run's clock, so no handler declaration of it exists.
                if handler.module == una_core::LIMIT_MODULE && handler.name == una_core::LIMITER {
                    let limiter = Rc::new(self.limiter(&values));
                    return Ok(Rc::new(Instance::Limiter { limiter, id }));
                }
                let info = self
                    .index
                    .handlers
                    .get(&(handler.module.as_str(), handler.name.as_str()))
                    .copied()
                    .ok_or_else(|| fault(format!("no handler `{}`", handler.qualified())))?;
                Ok(Rc::new(Instance::Handled {
                    handler: info,
                    values,
                    caps: restrict(caps, info.uses),
                    id,
                }))
            }
        }
    }

    /// The bucket of an installation of `limiter` (ADR 0117 rule 9, `spec/27 §10`) from its
    /// arguments `at_most`, `per` and `waiting`, on the run's clock (ADR 0116 rule 19). The
    /// installation's obligations give `at_most >= 1`, `per >= 1` and `waiting >= 0`; a count
    /// beyond 2^64 − 1 saturates, so a larger one never wraps into a smaller limit.
    fn limiter(&self, values: &[Value<'p>]) -> una_http::limit::Limiter {
        let count = |i: usize| match values.get(i) {
            Some(Value::Int(n)) => {
                u64::try_from(n).unwrap_or(if n.sign() == Sign::Minus { 0 } else { u64::MAX })
            }
            _ => 0,
        };
        una_http::limit::Limiter::new(
            count(0),
            count(1),
            count(2),
            Arc::new(self.run.clock.clone()),
        )
    }

    /// `acquire` of a `limiter` (ADR 0117 rule 9, `spec/27 §10`): takes a token, waiting for
    /// one as a request waits at a limit (`spec/27 §9` rule 4), within the deadlines of the
    /// `within`s in progress and of the `for ever within` iteration, interrupted by a host stop
    /// in the body of a `for ever`. Past a covering deadline, or on a host stop, no token is
    /// taken and the call fails with `RateLimited`, the one failure `Limited` has; so does a
    /// call that finds `waiting` calls already waiting. Under virtual time a wait for a refill
    /// moves the clock, to the refill or to the deadline, and takes no wall time.
    fn acquire(&mut self, limiter: &una_http::limit::Limiter) -> Flow<'p> {
        use una_http::client::Error;
        use una_http::limit::Abandon;
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
        let now = self.run.clock.now();
        // The covering deadline on the run's clock, which the wait reaches at most; on the wall
        // clock the same time, unless time is virtual, where only the run's clock bounds it.
        let until = left.map_or(u64::MAX, |left| {
            now.saturating_add(u64::try_from(left.as_millis()).unwrap_or(u64::MAX))
        });
        let wall = std::time::Instant::now();
        let deadline = match left {
            Some(left) if !self.run.clock.is_virtual() => wall.checked_add(left),
            _ => wall.checked_add(std::time::Duration::from_secs(365 * 24 * 3600)),
        }
        .unwrap_or(wall);
        // The interpreter runs one task at a time, so nothing abandons the wait but its
        // deadline or a host stop.
        let acquired = if self.in_ever {
            limiter.acquire((deadline, until), &self.stop.interrupt(), &Abandon::new())
        } else {
            limiter.acquire(
                (deadline, until),
                &una_http::client::Uninterrupted,
                &Abandon::new(),
            )
        };
        match acquired {
            Ok(()) => Ok(Value::Unit),
            Err(Error::TimedOut) => {
                // Only a covering deadline ends the wait with a time-out, and the call returns
                // once the run's clock has reached it, as natively (`spec/27 §5` rule 6, #1355).
                if left.is_some() && !self.run.clock.is_virtual() {
                    self.run.clock.wait_until(until);
                }
                // A `for ever within` iteration observes its deadline at the operation.
                if by_ever {
                    self.timeout_event();
                }
                Err(rate_limited())
            }
            Err(_) => Err(rate_limited()),
        }
    }

    fn atom(&self, env: &Env<'p>, atom: &'p Atom) -> Flow<'p> {
        Ok(match atom {
            Atom::Var(name) => env
                .lookup(name)
                .cloned()
                .ok_or_else(|| fault(format!("`{name}` is not bound")))?,
            Atom::Int(digits) => Value::Int(
                digits
                    .parse::<BigInt>()
                    .map_err(|_| fault(format!("`{digits}` is not an integer")))?,
            ),
            Atom::F64(bits) => Value::F64(una_core::float::from_bits(*bits)),
            Atom::Decimal { coefficient, scale } => Value::Decimal(
                coefficient
                    .parse::<BigInt>()
                    .map_err(|_| fault(format!("`{coefficient}` is not a coefficient")))?,
                *scale,
            ),
            Atom::F32(bits) => Value::F32(una_core::float32::from_bits(*bits)),
            Atom::Bool(b) => Value::Bool(*b),
            Atom::Text(text) => Value::text(text),
            Atom::Unit => Value::Unit,
        })
    }

    /// The iterations of a loop from the state `current` (`spec/17 §4`): iterations are not
    /// calls, so they do not count toward the call depth (ADR 0056, amendment of 2026-10-02).
    fn iterations(
        &mut self,
        env: &mut Env<'p>,
        caps: &Caps<'p>,
        (source, var, state, body, span): LoopParts<'p>,
        mut current: Vec<Value<'p>>,
    ) -> Result<Vec<Value<'p>>, Stop<'p>> {
        match source {
            // `ever`: until `stop`, a failure, or a host stop at the start of an iteration
            // (`spec/26 §5`, ADR 0116 rules 13 and 14). With `within d`, each iteration has the
            // deadline `d` after its start, which every interruptible operation it waits on or
            // starts honors (`spec/26 §7`).
            LoopSource::Ever { within } => {
                let period = match within {
                    Some((deadline, _)) => match self.atom(env, deadline)? {
                        // A deadline below 1 is refuted by verification; one that does not
                        // fit `u64` never passes.
                        Value::Int(n) => Some(if n.sign() == Sign::Minus {
                            0
                        } else {
                            u64::try_from(&n).unwrap_or(u64::MAX)
                        }),
                        _ => return Err(fault("a deadline that is not an `Int`")),
                    },
                    None => None,
                };
                let outer = self.deadline.take();
                // The iterations this instance of the loop has started (rule 19, #1472).
                let mut started: u64 = 0;
                let ended = loop {
                    if self.ever_start(&mut started) {
                        self.stop_event();
                        break Ok(());
                    }
                    self.run.tick();
                    if let Err(stop) = self.within_budget(span) {
                        break Err(stop);
                    }
                    self.in_ever = true;
                    self.deadline =
                        period.map(|ms| Deadline::after(ms, self.task.clone(), &self.run.clock));
                    let taken = std::mem::take(&mut current);
                    match self.iteration(env, caps, var, state, taken, &Value::Unit, body) {
                        Ok((next, stopped)) => {
                            current = next;
                            if stopped {
                                break Ok(());
                            }
                        }
                        Err(stop) => break Err(stop),
                    }
                };
                self.in_ever = false;
                self.deadline = outer;
                ended?;
            }
            // `until(c)`: `c` with the state bound, before each iteration (`spec/26 §4`).
            LoopSource::Until { cond, .. } => loop {
                self.run.tick();
                self.within_budget(span)?;
                let mark = env.len();
                for ((name, _), value) in state.iter().zip(&current) {
                    env.push((name.as_str(), value.clone()));
                }
                let holds = self.eval(env, caps, cond);
                env.truncate(mark);
                match holds? {
                    Value::Bool(true) => break,
                    Value::Bool(false) => {}
                    _ => return Err(fault("an `until` condition that is not a `Bool`")),
                }
                let stopped;
                (current, stopped) =
                    self.iteration(env, caps, var, state, current, &Value::Unit, body)?;
                if stopped {
                    break;
                }
            },
            LoopSource::List(list) => {
                let Value::List(items) = self.atom(env, list)? else {
                    return Err(fault("a loop over a value that is not a list"));
                };
                for item in items.iter() {
                    self.run.tick();
                    self.within_budget(span)?;
                    let stopped;
                    (current, stopped) =
                        self.iteration(env, caps, var, state, current, item, body)?;
                    if stopped {
                        break;
                    }
                }
            }
            LoopSource::Range {
                low,
                high,
                inclusive,
            } => {
                let (Value::Int(low), Value::Int(high)) =
                    (self.atom(env, low)?, self.atom(env, high)?)
                else {
                    return Err(fault("a loop over a range of non-integers"));
                };
                let mut index = low;
                loop {
                    let visit = if *inclusive {
                        index <= high
                    } else {
                        index < high
                    };
                    if !visit {
                        break;
                    }
                    let item = Value::Int(index.clone());
                    self.run.tick();
                    self.within_budget(span)?;
                    let stopped;
                    (current, stopped) =
                        self.iteration(env, caps, var, state, current, &item, body)?;
                    if stopped {
                        break;
                    }
                    // The successor of the upper bound is never computed, so a
                    // range up to the greatest value of a type stops there.
                    if index == high {
                        break;
                    }
                    index += 1;
                }
            }
        }
        Ok(current)
    }

    /// Whether the loop of the host, or a `for ever`, stops at the iteration that starts now
    /// (ADR 0116 rules 13, 14 and 19, `spec/26 §6`): a host stop request has reached the run,
    /// which is sticky; or this loop instance, which has started `started` iterations, has
    /// used up the budget of `--stop-after-iterations` or of a test's `stop after N`.
    /// Otherwise the iteration counts as started, on `started`.
    ///
    /// The budget is per loop instance (#1472, S-36): each entry into a `for ever`, and the
    /// driver's loop, counts its own iterations from 0, and the end of one instance's budget
    /// stops that instance only. It is not a host stop request: it interrupts no operation,
    /// stays with no later loop and starts no grace period. A count shared across loops, or a
    /// sticky stop it made, would let the iterations of a loop under a secret branch move where
    /// a later public loop stops.
    pub(crate) fn ever_start(&mut self, started: &mut u64) -> bool {
        if self.stop.stopped() {
            return true;
        }
        if self
            .run
            .stop_after
            .get()
            .is_some_and(|limit| *started >= limit)
        {
            return true;
        }
        *started = started.saturating_add(1);
        false
    }

    /// Appends the `stop` event of a loop that stops at the start of an iteration (ADR 0116
    /// rule 14), in the current task; none inside a `quiet` expression, whose program counter
    /// the event would reveal (S-39, `spec/26 §6` rule 4). It then takes no `seq` either.
    pub(crate) fn stop_event(&mut self) {
        if self.quiet > 0 {
            return;
        }
        let task = self.task.join(".");
        self.emit(Event {
            seq: 0,
            task,
            instance: String::new(),
            capability: String::new(),
            op: String::new(),
            args: Vec::new(),
            outcome: EventOutcome::Stop,
        });
    }

    /// The `on stop` block of a loop with the state `current` bound; returns the state it gives
    /// (`spec/26 §5`).
    fn on_stop(
        &mut self,
        env: &mut Env<'p>,
        caps: &Caps<'p>,
        on_stop: &'p una_core::OnStop,
        current: Vec<Value<'p>>,
    ) -> Result<Vec<Value<'p>>, Stop<'p>> {
        let mark = env.len();
        for ((name, _), value) in on_stop.state.iter().zip(current) {
            env.push((name.as_str(), value));
        }
        let result = self.eval(env, caps, &on_stop.body);
        env.truncate(mark);
        match result? {
            Value::Tuple(items) if items.len() == on_stop.state.len() => Ok(items.as_ref().clone()),
            _ => Err(fault("an `on stop` block whose value is not its state")),
        }
    }

    /// One iteration of a loop: the body with the loop variable and the current state bound;
    /// returns the new state, and whether a `stop` ended the loop (`spec/26 §3`).
    #[allow(clippy::too_many_arguments)]
    fn iteration(
        &mut self,
        env: &mut Env<'p>,
        caps: &Caps<'p>,
        var: &'p (Name, Type),
        state: &'p [(Name, Type)],
        current: Vec<Value<'p>>,
        item: &Value<'p>,
        body: &'p Expr,
    ) -> Result<(Vec<Value<'p>>, bool), Stop<'p>> {
        let mark = env.len();
        env.push((var.0.as_str(), item.clone()));
        for ((name, _), value) in state.iter().zip(current) {
            env.push((name.as_str(), value));
        }
        let result = self.eval(env, caps, body);
        env.truncate(mark);
        match result {
            Ok(Value::Tuple(items)) if items.len() == state.len() => Ok((
                Rc::try_unwrap(items).unwrap_or_else(|shared| (*shared).clone()),
                false,
            )),
            Err(Stop::Exit(tag, values)) if values.len() == state.len() => {
                Ok((values, tag == ExitTag::Stop))
            }
            Err(Stop::Exit(..)) | Ok(_) => Err(fault("a loop body whose value is not its state")),
            Err(stop) => Err(stop),
        }
    }

    /// Task `index` of a `together` block, `task`, with its path (`spec/19 §6` rule 1).
    /// Its events wait for the block's join, in the part it returns.
    fn task(
        &mut self,
        env: &mut Env<'p>,
        caps: &Caps<'p>,
        index: usize,
        task: &'p Expr,
    ) -> (Flow<'p>, Part) {
        self.task.push((index + 1).to_string());
        self.races.push(crate::concurrency::BlockCounts::default());
        self.tasks_runs
            .push(crate::concurrency::BlockCounts::default());
        self.buffers.push(Buffer {
            start: self.events.len(),
            path: self.task.join("."),
            installs: BTreeMap::new(),
        });
        // A task is a task of its own: a host stop reaches it only through its own `for ever`
        // loops (ADR 0116 rule 14).
        let in_ever = std::mem::replace(&mut self.in_ever, false);
        let mark = env.len();
        let result = self.eval(env, caps, task);
        env.truncate(mark);
        self.in_ever = in_ever;
        let buffer = self.buffers.pop().unwrap_or_default();
        let part = Part {
            buffer,
            end: self.events.len(),
        };
        self.tasks_runs.pop();
        self.races.pop();
        self.task.pop();
        (result, part)
    }

    /// Branch `index` of the `first of` named `race` in the task in progress, `task`, with its
    /// path `p.<race>.<index + 1>`, `race` being `f<c>` or, in quiet code, `qf` (`spec/27
    /// §6`), and the depth limit of `--task-depth` below its block, within the run's.
    fn branch(
        &mut self,
        env: &mut Env<'p>,
        caps: &Caps<'p>,
        (race, index): (&str, usize),
        task: &'p Expr,
    ) -> (Flow<'p>, Part) {
        self.task.push(race.to_owned());
        let max_depth = self.max_depth;
        self.max_depth = max_depth.min(self.depth.saturating_add(self.task_depth));
        let ended = self.task(env, caps, index, task);
        self.max_depth = max_depth;
        self.task.pop();
        ended
    }

    /// `tasks(b) e` (`spec/27 §7`, ADR 0117 rule 7): the block `body` runs in this task, and
    /// its `start`s start the tasks of the statement, at most `bound` at once, by default
    /// `max(64, M)` for the budget `M` in MiB, within the system's cap. The statement ends once
    /// the block has ended and every task it started has ended: a block that fails, or stops,
    /// first cancels them; one that ends, or leaves by `stop` or `skip` of an enclosing loop,
    /// lets them run to their end.
    fn tasks(
        &mut self,
        env: &mut Env<'p>,
        caps: &Caps<'p>,
        bound: Option<usize>,
        body: &'p Expr,
    ) -> Flow<'p> {
        self.tasks_with(bound, |machine| machine.eval(env, caps, body))
    }

    /// [`Machine::tasks`] with the block `body`, code of the engine's own: the driver of a
    /// `serve` section (`spec/31 §7` rule 4) runs its loop as the block of a `tasks` statement.
    pub(super) fn tasks_with(
        &mut self,
        bound: Option<usize>,
        body: impl FnOnce(&mut Self) -> Flow<'p>,
    ) -> Flow<'p> {
        // Quiet code does not count its statements, `qs`, so that whether it ran one, which may
        // depend on data above `public`, moves the path of no later statement (S-39, S-40,
        // `spec/26 §6` rule 4).
        let quiet = self.quiet > 0;
        let count = match self.tasks_runs.last_mut() {
            Some(counts) => counts.next(quiet),
            None => return Err(fault("a `tasks` statement outside any task")),
        };
        let bound = bound
            .unwrap_or_else(|| crate::concurrency::default_bound(self.max_memory, self.run.cap));
        let mut path = self.task.clone();
        path.push(crate::concurrency::block_name('s', quiet, count));
        let scope = Rc::new(TasksScope::new(path, bound));
        self.scopes.push(Rc::clone(&scope));
        let result = body(self);
        self.scopes.pop();
        if !matches!(result, Ok(_) | Err(Stop::Exit(..))) {
            scope.cancelled.set(true);
            // The held tasks of a block that fails or stops never run: dropped, each ends for
            // the statement as a cancelled task that reached no check point, quiet, with no
            // event (#1545, `spec/27 §7` rule 4).
            self.drop_held(&scope);
        }
        self.flush();
        // The tasks the statement does not hold first, as if quiet code had started none, then
        // the held ones, on this task's turn (#1545).
        self.wait_for(&scope, TasksScope::unheld_ended)?;
        while !scope.held.borrow().is_empty() {
            self.run_held(&scope)?;
        }
        self.run.ends_holding(&scope);
        self.wait_for(&scope, |scope| scope.running.get() == 0)?;
        // A task's abort that ends the run ends the statement with it.
        if let Some(abort) = self.run.aborted() {
            return Err(Stop::Abort(abort));
        }
        result
    }

    /// Parks this task until `done` holds of `scope`: a task of the statement that ends wakes
    /// it, and it checks again (ADR 0117 rule 3). A wait outside the scheduler, which only a
    /// defect reaches, is a fault rather than a hang.
    fn wait_for(
        &mut self,
        scope: &TasksScope,
        done: impl Fn(&TasksScope) -> bool,
    ) -> Result<(), Stop<'p>> {
        while !done(scope) {
            // A park is one of the task's switches: the tasks it holds run first (#1545).
            self.run.at_switch();
            if done(scope) {
                break;
            }
            scope.waiter.set(Some(self.run.current()));
            if !self.run.park() {
                scope.waiter.set(None);
                return Err(fault(
                    "a task waits for the tasks it started outside the scheduler",
                ));
            }
        }
        Ok(())
    }

    /// Runs the first task that `scope` holds to its end, on this task's turn (#1545, `spec/27
    /// §7` rule 4): it goes to the front of the queue with a turn count of its own, and once it
    /// has ended this task goes on, on the same turn, with the count its turn had before, unless
    /// the held task's turn ended meanwhile. A held task that ends alone, too deep, ends here as
    /// any started task does. O(1) besides the held task's own work.
    fn run_held(&mut self, scope: &TasksScope) -> Result<(), Stop<'p>> {
        let Some(id) = scope.held.borrow_mut().pop_front() else {
            return Ok(());
        };
        self.flush();
        let mark = {
            let mut sched = self.run.sched.borrow_mut();
            sched.push_front(id);
            sched.hold_turn()
        };
        scope.draining.set(Some(id));
        self.run.unholds();
        self.run.runs_inline(id);
        // The held task is part of this task's turn: it waits for it without a switch of its
        // own, so it runs none of its other held tasks meanwhile.
        let mut waited = Ok(());
        while scope.draining.get() == Some(id) {
            scope.waiter.set(Some(self.run.current()));
            if !self.run.park_inline() {
                scope.waiter.set(None);
                waited = Err(fault(
                    "a task waits for a task it holds outside the scheduler",
                ));
                break;
            }
        }
        scope.draining.set(None);
        self.run.sched.borrow_mut().resume_turn(mark);
        waited?;
        if let Some(abort) = self.run.aborted() {
            return Err(Stop::Abort(abort));
        }
        Ok(())
    }

    /// Drops the tasks `scope` holds, unrun: each one's end counts it out of the statement.
    fn drop_held(&mut self, scope: &TasksScope) {
        let held: Vec<usize> = scope.held.borrow_mut().drain(..).collect();
        for id in held {
            self.run.unholds();
            let body = self.run.sched.borrow_mut().unhold(id);
            drop(body);
        }
    }

    /// `start e` at `span` (`spec/27 §7`, ADR 0117 rule 7): once fewer tasks of the innermost
    /// statement run than its bound allows, starts the next one, `p.s<c>.<k>`, or `p.s<c>.q` in
    /// quiet code (S-40), for `task` with
    /// the bindings it reads and the capabilities in scope, at the back of the run's queue,
    /// and goes on. On Linux, a run already holding as many started tasks as the system allows
    /// at once aborts with U6003 at the `start`, before any mapping fails (ADR 0117 rule 2).
    fn start(
        &mut self,
        env: &Env<'p>,
        caps: &Caps<'p>,
        (task, rights): (&'p Expr, &'p [Name]),
        span: Span,
    ) -> Result<(), Stop<'p>> {
        let reads = self.reads(task);
        let mut captured: Env<'p> = reads.iter().filter_map(|name| env.binding(name)).collect();
        // What the walk of ADR 0123 rule 4.2 reads: the captures that can hold a reply right.
        let moved: Env<'p> = if rights.is_empty() || self.run.serve.is_none() {
            Env::new()
        } else {
            rights
                .iter()
                .filter_map(|name| captured.binding(name))
                .collect()
        };
        let caps = caps.clone();
        self.start_with(
            span,
            move |child| child.eval(&mut captured, &caps, task),
            // ADR 0123 rule 4.2: the reply rights this task holds that the captures reach move
            // to the started task, before it can run.
            |machine, id| machine.move_rights(&moved, rights, id),
        )
    }

    /// [`Machine::start`] of a task that runs `body`, code of the engine's own or the started
    /// expression, after `moves` gave it, by its id, the reply rights it takes (ADR 0123 rule
    /// 4.2), before it can run.
    pub(super) fn start_with(
        &mut self,
        span: Span,
        body: impl FnOnce(&mut Machine<'p, 'w>) -> Flow<'p> + 'w,
        moves: impl FnOnce(&Self, u64),
    ) -> Result<(), Stop<'p>> {
        let Some(scope) = self.scopes.last().cloned() else {
            return Err(fault("a `start` outside its `tasks` statement"));
        };
        self.flush();
        // A `start` in quiet code never waits at the bound: its task is held, and the bound
        // counts only the tasks the statement does not hold, so whether quiet code started
        // tasks never makes a public `start` wait (#1545, `spec/27 §7` rule 4). A statement
        // holds at most as many tasks as its bound: one more first runs the first one held.
        let quiet = self.quiet > 0;
        if quiet {
            while scope.held.borrow().len() >= scope.bound {
                self.run_held(&scope)?;
            }
        } else {
            self.wait_for(&scope, |scope| scope.bounded() < scope.bound)?;
        }
        if let Some(abort) = self.run.aborted() {
            return Err(Stop::Abort(abort));
        }
        if let Some(cap) = self.run.cap
            && self.run.live.get() >= cap
        {
            return Err(Stop::Abort(Abort::Unstarted {
                span: Some(span),
                cap: Some(cap),
            }));
        }
        // A `start` in quiet code is not counted, and its task is `q`, with no number, so that
        // whether it ran, which may depend on data above `public`, moves neither the index of a
        // later public one nor the path of a later quiet one (S-40, #1512, `spec/27 §7` rule 5).
        let mut counts = scope.started.get();
        let k = counts.next(quiet);
        scope.started.set(counts);
        let mut child = self.child(&scope, crate::concurrency::start_name(quiet, k));
        let bytes = crate::stack_bytes(self.task_depth).unwrap_or(STACK_RESERVE * 2);
        scope.running.set(scope.running.get() + 1);
        self.run.live.set(self.run.live.get() + 1);
        // The started task's id, which its reply rights are recorded under (ADR 0123 rule 4).
        let id = self.run.sched.borrow().upcoming();
        let held_scope = Rc::clone(&scope);
        let ending = Ending {
            scope,
            run: Rc::clone(&self.run),
            task: id,
        };
        let task = move || {
            // Its statement counts it ended once its events are written, or, if it never
            // starts, when the driver drops it, or after a panic, which the run keeps.
            let _ending = ending;
            let run = Rc::clone(&child.run);
            run.guarded(|| {
                let base = 0usize;
                child.stack_base = stack_position(&base);
                let result = body(&mut child);
                child.started_end(result);
            });
        };
        moves(self, id as u64);
        let started = if quiet {
            let held = self
                .run
                .sched
                .borrow_mut()
                .hold(Box::new(task), bytes, span);
            held_scope.held.borrow_mut().push_back(held);
            self.run.holds(&held_scope);
            held
        } else {
            self.run
                .sched
                .borrow_mut()
                .start(Box::new(task), bytes, span)
        };
        if started != id {
            return Err(fault("a started task took an id other than its own"));
        }
        Ok(())
    }

    /// The end of a started task with `result` (`spec/27 §7`): one that ran too deep ends
    /// alone, with its `aborted` event and its line on stderr, and the other tasks run on; one
    /// that was cancelled, by its statement or a deadline in force when it started, ends with
    /// its `cancelled` event; any other abort ends the whole run. Then its events are written.
    fn started_end(&mut self, result: Flow<'p>) {
        let task = self.task.join(".");
        let ended = |task: String, outcome: EventOutcome| Event {
            seq: 0,
            task,
            instance: String::new(),
            capability: String::new(),
            op: String::new(),
            args: Vec::new(),
            outcome,
        };
        self.ops_open = 0;
        // The reply rights the task still holds are answered first, each with its `unanswered`
        // event, then the event of how it ended (ADR 0123 rule 4.5).
        self.answer_held(&result, true);
        match result {
            Ok(_) => {}
            Err(Stop::Abort(abort @ Abort::Depth { .. })) => {
                self.run.world.borrow_mut().task_aborted(&task, &abort);
                self.emit(ended(task, EventOutcome::TaskAborted));
            }
            Err(Stop::Abort(abort)) => self.run.abort_run(abort),
            // None inside a `quiet` expression, which the task started under (S-39, `spec/26 §6`
            // rule 4).
            Err(Stop::Cut(_)) if self.quiet > 0 => {}
            Err(Stop::Cut(_)) => self.emit(ended(task, EventOutcome::Cancelled)),
            // A started task cannot fail (U3029), and no exit leaves its expression.
            Err(Stop::Fail(_) | Stop::Exit(..)) => self.run.abort_run(Abort::Fault {
                message: "a started task failed, which its checks exclude".to_owned(),
                span: self.frames.last().copied(),
            }),
        }
        self.finish_trace();
    }

    /// `first of` (`spec/27 §6`, ADR 0117 rule 6): its branches run one after another, in index
    /// order, and the first that returns wins, one legal schedule; no branch after it starts.
    /// When every branch fails, the race fails with the failure of the first. The events of
    /// the winner come first, then those of the branches before it, in index order, each
    /// renumbered as native code numbers them at the join, the ids of the handlers they
    /// installed included.
    fn first_of(&mut self, env: &mut Env<'p>, caps: &Caps<'p>, tasks: &'p [Expr]) -> Flow<'p> {
        // Quiet code does not count its races, `qf`, as `tasks` statements (S-39, S-40).
        let quiet = self.quiet > 0;
        let race = match self.races.last_mut() {
            Some(counts) => crate::concurrency::block_name('f', quiet, counts.next(quiet)),
            None => return Err(fault("a `first of` outside any task")),
        };
        let start = self.events.len();
        let mut parts: Vec<Part> = Vec::with_capacity(tasks.len());
        let mut failure = None;
        for (index, task) in tasks.iter().enumerate() {
            // The start of each branch is a check point for a cut (`spec/27 §5`).
            let result = match self.passed_within(self.within_shield) {
                Some(id) => Err(Stop::Cut(id)),
                None => {
                    let (result, part) = self.branch(env, caps, (&race, index), task);
                    parts.push(part);
                    result
                }
            };
            match result {
                Ok(value) => {
                    // The winner's events first, then those of the branches before it.
                    let mut order: Vec<&Part> = Vec::with_capacity(parts.len());
                    order.extend(parts.last());
                    order.extend(parts.iter().take(parts.len().saturating_sub(1)));
                    self.adopt(start, &order);
                    return Ok(value);
                }
                Err(Stop::Fail(error)) => {
                    failure.get_or_insert(error);
                }
                Err(stop) => {
                    self.adopt(start, &parts.iter().collect::<Vec<_>>());
                    if matches!(stop, Stop::Cut(_)) {
                        self.race_cut_events(&race, index, tasks.len());
                    }
                    return Err(stop);
                }
            }
        }
        self.adopt(start, &parts.iter().collect::<Vec<_>>());
        match failure {
            Some(error) => Err(Stop::Fail(error)),
            None => Err(fault("a `first of` without branches")),
        }
    }

    /// The elements a `together for` visits, in order.
    fn elements(
        &mut self,
        env: &Env<'p>,
        source: &'p LoopSource,
        span: Span,
    ) -> Result<Vec<Value<'p>>, Stop<'p>> {
        match source {
            LoopSource::List(list) => match self.atom(env, list)? {
                Value::List(items) => Ok(items.as_ref().clone()),
                _ => Err(fault("a `together for` over a value that is not a list")),
            },
            LoopSource::Until { .. } | LoopSource::Ever { .. } => {
                Err(fault("a `together for` with an `until` or `ever` source"))
            }
            LoopSource::Range {
                low,
                high,
                inclusive,
            } => {
                let (Value::Int(low), Value::Int(high)) =
                    (self.atom(env, low)?, self.atom(env, high)?)
                else {
                    return Err(fault("a `together for` over a range of non-integers"));
                };
                let mut items = Vec::new();
                let mut index = low;
                loop {
                    let visit = if *inclusive {
                        index <= high
                    } else {
                        index < high
                    };
                    if !visit {
                        break;
                    }
                    self.within_budget(span)?;
                    items.push(Value::Int(index.clone()));
                    // The successor of the upper bound is never computed (`spec/17 §7`).
                    if index == high {
                        break;
                    }
                    index += 1;
                }
                Ok(items)
            }
        }
    }

    fn atoms(&self, env: &Env<'p>, atoms: &'p [Atom]) -> Result<Vec<Value<'p>>, Stop<'p>> {
        atoms.iter().map(|a| self.atom(env, a)).collect()
    }

    /// A right-hand side, bound to a binder of type `ty`.
    fn rhs(&mut self, env: &Env<'p>, caps: &Caps<'p>, rhs: &'p Rhs, ty: &Type) -> Flow<'p> {
        match rhs {
            Rhs::Atom(atom) => self.atom(env, atom),
            Rhs::Prim {
                prim, args, span, ..
            } => {
                let args = self.atoms(env, args)?;
                // A hash table's operations compare keys within the memory budget, as `==`
                // does (`spec/28 §7`).
                if matches!(prim, Prim::HashMap(_) | Prim::HashSet(_)) {
                    return self.hash_apply(*prim, args, *span);
                }
                if let (Prim::Eq | Prim::Ne, [a, b]) = (*prim, args.as_slice()) {
                    let same = self.equal_at(a, b, *span)?;
                    return Ok(Value::Bool(same == (*prim == Prim::Eq)));
                }
                // Interpolation writes a structured value in its Una form (ADR 0068).
                if *prim == Prim::ToText
                    && let Some(value) = args.first()
                    && !matches!(value, Value::Int(_) | Value::Bool(_) | Value::Text(_))
                {
                    let text = self
                        .index
                        .written(value, &|bytes| self.fits(bytes))
                        .map_err(|OverBudget| self.over_budget(*span))?;
                    return Ok(Value::text(&text));
                }
                // A large `Int` is written once its text fits the budget (`spec/11 §7`, #1223).
                if *prim == Prim::ToText
                    && let Some(Value::Int(n)) = args.first()
                    && una_core::int_text_room(n).is_some_and(|bytes| !self.fits(bytes))
                {
                    return Err(self.over_budget(*span));
                }
                // So is a `Decimal`'s, from its coefficient.
                if let Prim::Decimal(DecimalOp::ToText(_) | DecimalOp::ToJson(_)) = *prim
                    && let Some(Value::Decimal(c, _)) = args.first()
                    && una_core::int_text_room(c).is_some_and(|bytes| !self.fits(bytes))
                {
                    return Err(self.over_budget(*span));
                }
                // A bit set's precondition aborts at the call (ADR 0119 rule 14).
                if let Prim::BitSet(op) = *prim {
                    return bitsets::bit_set_apply(op, args, Some(*span));
                }
                prim_apply(*prim, &args)
            }
            Rhs::Call {
                function,
                args,
                span,
                ..
            } => {
                let args = self.atoms(env, args)?;
                self.call(function, args, caps, *span)
            }
            Rhs::Apply {
                callee, args, span, ..
            } => {
                let callee = self.atom(env, callee)?;
                let args = self.atoms(env, args)?;
                self.apply(&callee, args, *span)
            }
            Rhs::Op {
                cap,
                capability,
                op,
                args,
                labels,
                outcome,
                span,
                ..
            } => {
                let args = self.atoms(env, args)?;
                let request = Request {
                    cap,
                    capability,
                    op,
                    labels,
                    result: ty.deep_label(),
                    outcome: outcome.join(&ty.deep_label()),
                    span: *span,
                };
                self.op(caps, &request, args)
            }
            Rhs::Construct { ctor, args, .. } => {
                Ok(Value::data(ctor.clone(), self.atoms(env, args)?))
            }
            Rhs::Field { value, index, .. } => match self.atom(env, value)? {
                Value::Data(data) => data
                    .fields
                    .get(*index)
                    .cloned()
                    .ok_or_else(|| fault("a field that the record does not have")),
                _ => Err(fault("a field of a value that is not a record")),
            },
            Rhs::List(items) => Ok(Value::List(Rc::new(self.atoms(env, items)?))),
            Rhs::Tuple(items) => Ok(Value::Tuple(Rc::new(self.atoms(env, items)?))),
            // A closure keeps, by value, the bindings its body reads: what a copy of the whole
            // environment would give it, at the cost of what it reads rather than of what is in
            // scope, at its making and at each call (#329).
            Rhs::Lambda { params, body, .. } => {
                let reads = self.reads(body);
                let captured: Env<'p> = reads.iter().filter_map(|name| env.binding(name)).collect();
                Ok(Value::Closure(Rc::new(Closure {
                    params: params.iter().map(|(name, _)| name).collect(),
                    body,
                    env: captured,
                    caps: caps.clone(),
                })))
            }
            Rhs::Checked {
                value,
                it,
                predicate,
                ctor,
                args,
                ..
            } => {
                let value = self.atom(env, value)?;
                let mut inner = env.clone();
                inner.push((it.as_str(), value.clone()));
                match self.eval(&mut inner, caps, predicate)? {
                    Value::Bool(true) => Ok(value),
                    Value::Bool(false) => Err(Stop::Fail(Value::data(
                        ctor.clone(),
                        self.atoms(env, args)?,
                    ))),
                    _ => Err(fault("a `checked` predicate that is not a `Bool`")),
                }
            }
            Rhs::Declassify { via, arg, span, .. } => {
                let arg = self.atom(env, arg)?;
                self.call(via, vec![arg], caps, *span)
            }
        }
    }

    /// E-Call: a declared function or predicate, or a function of the prelude or of
    /// `std.testing`.
    fn call(
        &mut self,
        function: &'p Path,
        args: Vec<Value<'p>>,
        caps: &Caps<'p>,
        span: Span,
    ) -> Flow<'p> {
        match function.module.as_str() {
            "prelude" => return self.prelude(&function.name, args, span),
            una_core::COLLECTIONS_MODULE => {
                if let Some(made) = self.collections(&function.name, &args, span) {
                    return made;
                }
            }
            "std.testing" if function.name == "fake_hash" => {
                return match args.first() {
                    Some(Value::Int(seed)) => Ok(Value::Hashed(Rc::from(host::fake_hash(seed)))),
                    _ => Err(fault("`fake_hash` without an `Int` seed")),
                };
            }
            // ADR 0123 rule 21: a request and an exchange for a test of a server.
            "std.testing" if function.name == "test_request" => {
                return serve::test_request(&args);
            }
            "std.testing" if function.name == "test_exchange" => {
                return self.test_exchange(&args);
            }
            // `elapse(d)` moves the test's virtual clock forward by `d` milliseconds at once
            // (ADR 0116 rule 19, `spec/14 §4`). Its `requires d >= 0` is verified at each call,
            // and only tests call it, whose clock is always virtual.
            "std.testing" if function.name == "elapse" => {
                let by = match args.first() {
                    Some(Value::Int(d)) if d.sign() != Sign::Minus => {
                        u64::try_from(d).unwrap_or(u64::MAX)
                    }
                    _ => return Err(fault("`elapse` without a duration of at least 0")),
                };
                return match self.run.clock.elapse(by) {
                    Ok(_) => Ok(Value::Unit),
                    Err(crate::clock::NotVirtual) => {
                        Err(fault("`elapse` outside a test, on the real clock"))
                    }
                };
            }
            _ => {}
        }
        let key = (function.module.as_str(), function.name.as_str());
        let (params, body, uses, frame): (Vec<&'p str>, &'p Expr, &'p [Use], Option<Span>) =
            if let Some(fun) = self.index.functions.get(&key) {
                let params = fun.params.iter().map(|(n, _)| n.as_str()).collect();
                (params, &fun.body, &fun.uses, Some(fun.span))
            } else if let Some((params, body)) = self.index.predicates.get(&key) {
                let params = params.iter().map(|(n, _)| n.as_str()).collect();
                (params, *body, &[], None)
            } else {
                return Err(fault(format!("no function `{}`", function.qualified())));
            };
        self.enter(span)?;
        if let Some(frame) = frame {
            self.frames.push(frame);
        }
        let mut env: Env<'p> = params.into_iter().zip(args).collect();
        let result = self.eval(&mut env, &restrict(caps, uses), body);
        let result = self.locate(result);
        if frame.is_some() {
            self.frames.pop();
        }
        self.leave(result)
    }

    /// A function of `std.collections` whose body is native (`spec/28`, ADR 0119), called with
    /// `args` at `span`, or `None` for one the module declares in Una.
    fn collections(&mut self, name: &str, args: &[Value<'p>], span: Span) -> Option<Flow<'p>> {
        Some(Ok(match name {
            // O(1): an empty ring buffer allocates nothing.
            "empty_stack" | "empty_queue" => Value::Seq(Rc::default()),
            // O(1).
            "empty_set" => Value::Set(Rc::default()),
            // O(1): a new table with a key of its own (`spec/28 §1` rule 4).
            "empty_hash_map" | "empty_hash_set" => {
                let key = match hashes::table_key(self.run.hashing.borrow().as_ref(), span) {
                    Ok(key) => key,
                    Err(stop) => return Some(Err(stop)),
                };
                let table = Rc::new(crate::value::HashTable::new(key));
                if name == "empty_hash_map" {
                    Value::HashMap(table)
                } else {
                    Value::HashSet(table)
                }
            }
            // O(1): an empty heap allocates nothing; the direction is part of the value.
            "lowest_first" => priority::empty(false),
            "highest_first" => priority::empty(true),
            // O(size / 64), within the memory budget (`spec/28 §9`).
            "empty_bit_set" => return Some(self.empty_bit_set(args, span)),
            // The conversions of a list (`spec/28 §10`).
            "to_stack" | "to_queue" | "to_set" => return Some(Self::conversion(name, args)),
            // The conversions into a hash table, under a key of its own (`spec/28 §1` rule 4).
            "to_hash_set" | "to_hash_map" => return Some(self.to_hash_table(name, args, span)),
            _ => return None,
        }))
    }

    /// The memory budget of the key comparisons of a hash table operation at `span`, as
    /// `equal_at` compares.
    fn hash_budget(&self, span: Span) -> hashes::Budget {
        hashes::Budget {
            pairs: self.max_memory.saturating_mul(1024 * 1024) / EQUALITY_PAIR_BYTES,
            span,
            limit: self.max_memory,
        }
    }

    /// `xs.to_hash_set()` or `es.to_hash_map()` at `span` (`spec/28 §10`, ADR 0119 rule 16): a
    /// new table under a key of its own, made as `empty_hash_set` and `empty_hash_map` make
    /// theirs, filled from the list. O(n · |k|) expected.
    fn to_hash_table(&self, name: &str, args: &[Value<'p>], span: Span) -> Flow<'p> {
        let key = hashes::table_key(self.run.hashing.borrow().as_ref(), span)?;
        hashes::from_list(name, key, args, &self.hash_budget(span))
    }

    /// A method of a hash map or a hash set at `span`, its keys compared by `==` within the
    /// memory budget, as `equal_at` compares (`spec/28 §7`).
    fn hash_apply(&self, prim: Prim, args: Vec<Value<'p>>, span: Span) -> Flow<'p> {
        let budget = self.hash_budget(span);
        match prim {
            Prim::HashMap(op) => hashes::map_apply(op, args, &budget),
            Prim::HashSet(op) => hashes::set_apply(op, args, &budget),
            _ => Err(fault("a hash table operation of another primitive")),
        }
    }

    /// A call of a function value.
    fn apply(&mut self, callee: &Value<'p>, args: Vec<Value<'p>>, span: Span) -> Flow<'p> {
        let Value::Closure(closure) = callee else {
            return Err(fault("a call of a value that is not a function"));
        };
        self.enter(span)?;
        let mut env = closure.env.clone();
        env.extend(closure.params.iter().map(|p| p.as_str()).zip(args));
        let result = self.eval(&mut env, &closure.caps, closure.body);
        self.leave(result)
    }

    /// E-OpHost and E-OpHandler: the operation's event comes first, then the events of a
    /// handler's body; its outcome is filled in when the operation ends.
    fn op(&mut self, caps: &Caps<'p>, request: &Request<'_>, args: Vec<Value<'p>>) -> Flow<'p> {
        let Request {
            cap,
            capability,
            op,
            labels,
            span,
            ..
        } = *request;
        // An operation is a check point of the task's turn (ADR 0117 rule 12).
        self.run.tick();
        // The tick may have let another task run: when its abort ended the whole run, this
        // operation does not start, as at every check point (`spec/27 §7`), so no `reply` or
        // other effect follows an abort of the run (ADR 0123 rule 4.7).
        if self.run.coroutines
            && let Some(abort) = self.run.aborted()
        {
            return Err(Stop::Abort(abort));
        }
        // A cancelled task, or a `within` whose deadline has passed, stops before an operation
        // starts, as native code polls before each one (`spec/27 §5`, §7, `spec/19 §6` rule 4).
        if self.cancelled() {
            return Err(Stop::Cut(CANCEL));
        }
        if let Some(id) = self.passed_within(self.within_shield) {
            return Err(Stop::Cut(id));
        }
        let instance = caps
            .get(cap)
            .cloned()
            .ok_or_else(|| fault(format!("no capability `@{cap}`")))?;
        // S-42 (#1515, `spec/26 §6` rule 4): an operation performed in quiet code appends no
        // event and takes no `seq`, so neither the number of events nor the `seq` of a later
        // one shows whether it ran; the events of a handler's code that serves it are in quiet
        // code too. Code quiet by a timing floor alone keeps them (`spec/27 §5` rule 7), and
        // an example's trace keeps them, for `expect performed` (`spec/08`).
        // #1544: an instance that quiet code installed, `with:<h>#q`, is in scope only in quiet
        // code, so its operations append no event in any trace; the handler's code runs as
        // before. Natively the runtime starts no event for either (`una_rt::host::event`).
        if (self.hushed > 0 && !self.run.quiet_events.get())
            || crate::concurrency::is_quiet_install(instance.id())
        {
            return self.serve(&instance, capability, op, args, span);
        }
        let names: Vec<String> = match self.index.capabilities.get(capability) {
            Some(ops) => ops
                .iter()
                .find(|o| o.name == op)
                .map(|o| o.params.iter().map(|(n, _)| source_name(n)).collect())
                .unwrap_or_default(),
            None => host_params(capability, op)
                .iter()
                .map(|n| (*n).to_owned())
                .collect(),
        };
        // ADR 0123 rule 1 (`spec/31 §1` rule 7): the values of the events of the host's
        // `HttpServer` are redacted by their labels, as every event's are, and are written in
        // the reduced form whatever their labels, under the run's trace key. Without the run's
        // engine, which a run that serves nothing lacks, every argument is written as an
        // `internal` value is, redacted, failing closed.
        let server = capability == "HttpServer" && matches!(*instance, Instance::Host { .. });
        let trace = if server {
            self.run.serve.as_ref().map(serve::Serving::trace)
        } else {
            None
        };
        let mut shown: Vec<(String, String)> = Vec::with_capacity(args.len());
        for (i, value) in args.iter().enumerate() {
            let name = names.get(i).cloned().unwrap_or_else(|| format!("arg{i}"));
            // An argument labeled above `public` is redacted.
            let label = labels.get(i).cloned().unwrap_or_else(Label::public);
            // A write that does not fit the budget aborts the run at the operation, before it
            // starts (`spec/11 §7`, #1223).
            let room = |bytes| self.fits(bytes);
            let text = match trace {
                // The toolchain's tests only: the reduced form without the labels' redaction.
                Some((key, true)) => self.index.server_form(value, Form::Written, &key, &room),
                Some((key, false)) => self.index.server_redacted(value, &label, &key, &room),
                None if server => self.index.redacted(
                    value,
                    &label.join(&Label {
                        level: Level::Internal,
                        ..Label::public()
                    }),
                    &room,
                ),
                None => self.index.redacted(value, &label, &room),
            }
            .map_err(|OverBudget| self.over_budget(span))?;
            shown.push((name, text));
        }
        // The operation's event is written when it ends, after the events of the handler code
        // that served it (ADR 0117 rule 10).
        let slot = self.events.len();
        let task = self.task.join(".");
        self.events.push(Event {
            seq: 0,
            task,
            instance: instance.id().to_owned(),
            capability: capability.to_owned(),
            op: op.to_owned(),
            args: shown,
            outcome: EventOutcome::Aborted,
        });
        self.ops_open += 1;
        let result = self.serve(&instance, capability, op, args, span);
        self.ops_open = self.ops_open.saturating_sub(1);
        // S-45 (#1552, `spec/11 §5`): in a run, an outcome labeled above `public` is written
        // as `redacted`, at the static label of the site, so whether it is depends on no
        // value; no text is built from its value.
        let revealed = matches!(trace, Some((_, true)));
        let hidden = (!request.outcome.is_public() && !self.run.clear_outcomes.get() && !revealed)
            .then(|| level_name(request.outcome.level));
        let (result, outcome) = match (result, hidden) {
            (result @ (Ok(_) | Err(Stop::Fail(_))), Some(level)) => {
                (result, EventOutcome::Redacted(level.to_owned()))
            }
            // An outcome that does not fit the budget aborts the run at the operation, its event
            // left `aborted` (`spec/11 §7`, #1223).
            (Ok(value), None) => {
                let room = |bytes| self.fits(bytes);
                let text = match trace {
                    Some((key, true)) => self.index.server_form(&value, Form::Written, &key, &room),
                    Some((key, false)) => {
                        self.index
                            .server_redacted(&value, &request.result, &key, &room)
                    }
                    None => self.index.redacted(&value, &request.result, &room),
                };
                match text {
                    Ok(text) => (Ok(value), EventOutcome::Ok(text)),
                    Err(OverBudget) => (Err(self.over_budget(span)), EventOutcome::Aborted),
                }
            }
            (Err(Stop::Fail(failure)), None) => {
                match self.index.render(&failure, &|bytes| self.fits(bytes)) {
                    Ok(text) => {
                        let variant = variant(&failure);
                        (
                            Err(Stop::Fail(failure)),
                            EventOutcome::Fail {
                                variant,
                                value: text,
                            },
                        )
                    }
                    Err(OverBudget) => (Err(self.over_budget(span)), EventOutcome::Aborted),
                }
            }
            // `next` of the host's `HttpServer`, an interruptible operation, was waiting when its
            // task was cancelled, and took no request (ADR 0117 rules 4 and 10, ADR 0123 rule
            // 4.7).
            (other @ Err(Stop::Cut(CANCEL)), _) if server && op == "next" => {
                (other, EventOutcome::Interrupted)
            }
            // An operation in progress completes (`spec/19 §6` rule 4): a cut inside its
            // handler is only one of a `within` that the handler's own code started, which
            // catches it there.
            (other @ Err(Stop::Abort(_) | Stop::Exit(..) | Stop::Cut(_)), _) => {
                (other, EventOutcome::Aborted)
            }
        };
        if let Some(event) = self.events.get_mut(slot) {
            event.outcome = outcome;
        }
        self.flush();
        result
    }

    /// The operation `op` of `instance`, with `args`: a host operation, or the code of a
    /// handler's operation, which completes whatever deadline or cancellation in force when it
    /// began passes meanwhile (`spec/19 §6` rule 4).
    fn serve(
        &mut self,
        instance: &Instance<'p>,
        capability: &str,
        op: &str,
        args: Vec<Value<'p>>,
        span: Span,
    ) -> Flow<'p> {
        match instance {
            Instance::Host { .. } => self.host(capability, op, &args, span),
            Instance::Limiter { limiter, .. } => {
                let limiter = Rc::clone(limiter);
                self.acquire(&limiter)
            }
            Instance::Handled {
                handler,
                values,
                caps: own,
                ..
            } => {
                let Some(implementation) = handler.ops.iter().find(|o| o.name == op) else {
                    return Err(fault(format!(
                        "handler `{}` has no operation `{op}`",
                        handler.name
                    )));
                };
                self.enter(span)?;
                let mut env: Env<'p> = handler
                    .params
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .zip(values.iter().cloned())
                    .collect();
                env.extend(implementation.params.iter().map(String::as_str).zip(args));
                let shield = std::mem::replace(&mut self.within_shield, self.withins.len());
                let cancel_shield = std::mem::replace(&mut self.cancel_shield, self.cancels.len());
                let result = self.eval(&mut env, own, &implementation.body);
                self.cancel_shield = cancel_shield;
                self.within_shield = shield;
                self.leave(result)
            }
        }
    }

    /// An operation failed because the deadline of the iteration in progress passed (ADR 0116
    /// rule 11). In the task that runs the loop, the iteration observes it here; in a task it
    /// started, at the join of that task's block ([`Self::joined`]).
    fn timeout_event(&mut self) {
        // Quiet code neither observes the deadline nor tells the join it did, so whether the
        // iteration's `timeout` event appears never depends on whether quiet code ran (S-39 L1,
        // S-42, `spec/26 §6` rule 4).
        if self.quiet > 0 {
            return;
        }
        let Some(deadline) = self.deadline.as_mut() else {
            return;
        };
        if self.task != deadline.owner {
            deadline.hit = true;
            return;
        }
        self.timeout_observed();
    }

    /// The join of a `together` or `together for` block: when the task that runs the loop
    /// joins a block one of whose tasks met the deadline, the iteration observes it here.
    fn joined(&mut self) {
        if self
            .deadline
            .as_ref()
            .is_some_and(|deadline| deadline.hit && self.task == deadline.owner)
        {
            self.timeout_observed();
        }
    }

    /// Appends the iteration's one `timeout` event, with the path of the task that runs the
    /// loop, the first time its control flow observes a failure the deadline caused: a trace
    /// does not depend on which task met the deadline first (`spec/26 §7`). None inside a
    /// `quiet` expression (S-39, `spec/26 §6` rule 4), which does not mark it as appended either.
    fn timeout_observed(&mut self) {
        // Quiet code does not consume the iteration's one event either: a later observation
        // in public code still appends it (S-39 L1).
        if self.quiet > 0 {
            return;
        }
        let Some(deadline) = self.deadline.as_mut() else {
            return;
        };
        if deadline.reported {
            return;
        }
        deadline.reported = true;
        let task = deadline.owner.join(".");
        self.emit(Event {
            seq: 0,
            task,
            instance: String::new(),
            capability: String::new(),
            op: String::new(),
            args: Vec::new(),
            outcome: EventOutcome::Timeout,
        });
    }

    /// An operation of `Http` (`spec/24`), by the client that native code shares.
    fn http(&mut self, op: &str, args: &[Value<'p>], span: Span) -> Flow<'p> {
        use una_http::client::Error;
        use una_http::grant::Method;
        let fail = |error: Error| {
            let (name, index) = match error {
                Error::Denied => ("Denied", 0),
                Error::BadUrl => ("BadUrl", 1),
                Error::BadRequest => ("BadRequest", 2),
                Error::Unreachable => ("Unreachable", 3),
                Error::TimedOut => ("TimedOut", 4),
                Error::TooLarge => ("TooLarge", 5),
                Error::Tls => ("Tls", 6),
                Error::Protocol => ("Protocol", 7),
                // Edition 0.4 only: no operation of an earlier edition runs in a `for ever`, and
                // limits need edition 0.4 (ADR 0117 rule 8, `spec/27 §9`).
                Error::Interrupted => ("Interrupted", 8),
                Error::RateLimited => ("RateLimited", 9),
                // Not a failure: the run aborts.
                Error::HeaderValue => return Stop::Abort(Abort::HeaderValue { span }),
            };
            Stop::Fail(Value::data(
                prelude_ctor("HttpError", name, index),
                Vec::new(),
            ))
        };
        let text = |value: Option<&Value<'p>>| match value {
            Some(Value::Text(t)) => t.to_string(),
            _ => String::new(),
        };
        let method =
            Method::of(op).ok_or_else(|| fault("an operation of `Http` with no method"))?;
        let with_body = matches!(method, Method::Post | Method::Put);
        let url = text(args.first());
        let headers: Vec<(String, String)> = match args.get(1) {
            Some(Value::List(items)) => items
                .iter()
                .map(|item| match item {
                    Value::Data(header) => {
                        (text(header.fields.first()), text(header.fields.get(1)))
                    }
                    _ => (String::new(), String::new()),
                })
                .collect(),
            _ => Vec::new(),
        };
        let body = if with_body {
            text(args.get(2))
        } else {
            String::new()
        };
        let key_name = match args.get(if with_body { 3 } else { 2 }) {
            Some(Value::Data(option)) if option.ctor.name == "Some" => {
                Some(text(option.fields.first()))
            }
            _ => None,
        };
        // The key a name stands for; an unknown name is `Denied` (`spec/24 §1` rule 3).
        let bearer = match key_name {
            Some(name) => Some(
                self.keys
                    .get(&name)
                    .ok_or_else(|| fail(Error::Denied))?
                    .clone(),
            ),
            None => None,
        };
        // The deadline of a `for ever within` iteration (`spec/26 §7`): past it, the request
        // fails at once with `TimedOut`; before it, it waits no longer than the time left. In
        // the body of a `for ever`, a host stop interrupts the request too (ADR 0116 rule 12):
        // whichever comes first cuts the wait, in the same poll.
        //
        // Under virtual time, a network wait takes no virtual time (ADR 0116 rule 19), so only a
        // deadline the virtual clock has already reached cuts the request, at once.
        // The deadlines of the `within`s in progress bound the wait the same way (`spec/27 §5`);
        // a cut by one is observed at the next check point, or when its expression returns.
        let mut settings = self.http;
        let mut cut = false;
        // The covering deadline on the run's clock, which a wait at a limit reaches at most: the
        // earliest deadline in force itself, and the time left before it from one reading of
        // the clock, so the two never differ by a millisecond. Under virtual time, a wait for a
        // refill moves the clock to it (ADR 0116 rule 19).
        let now = self.run.clock.now();
        let ever = self.deadline.as_ref().and_then(|deadline| deadline.at);
        let within = self.withins.iter().filter_map(|(_, at)| *at).min();
        let (cover, by_ever) = match (ever, within) {
            (Some(ever), Some(within)) if within < ever => (Some(within), false),
            (Some(ever), _) => (Some(ever), true),
            (None, within) => (within, false),
        };
        let left = cover.map(|at| std::time::Duration::from_millis(at.saturating_sub(now)));
        if let Some(left) = left
            && left < settings.timeout
            && (left.is_zero() || !self.run.clock.is_virtual())
        {
            settings.timeout = left;
            cut = true;
        }
        let owned = Owned {
            method,
            url,
            headers,
            body: body.into_bytes(),
            bearer,
        };
        // In the body of a `for ever`, a host stop interrupts the request (ADR 0116 rule 12).
        let stop = self.in_ever.then(|| Arc::clone(&self.stop));
        let run = Rc::clone(&self.run);
        let sent = if cut && settings.timeout.is_zero() {
            Err(Error::TimedOut)
        } else if let Some(offload) = run.offloads()
            // A header value with a control character aborts the run at the operation (U6004,
            // `spec/24 §1`): such a request takes no place, and its `send` here aborts it.
            && !una_http::client::bad_header_value(&owned.headers)
            && run.take_place(offload)
        {
            self.http_off_thread(offload, owned, settings, cover, stop)?
        } else {
            // On the run's thread, as before #1730: in a run that serves live peers, when every
            // place of `spec/27 §2` rule 3 is held. The run's limits are shared by its tasks
            // (ADR 0117 rule 8), and nothing abandons a wait at one but its deadline or a host
            // stop.
            send(&owned, &run.http_limits, &settings, stop.as_deref(), cover)
        };
        // Under virtual time, a wait at a limit that moved the clock to the covering deadline
        // was cut by it, as a request is under the real clock.
        if !cut
            && sent == Err(Error::TimedOut)
            && self.run.clock.is_virtual()
            && cover.is_some_and(|at| self.run.clock.now() >= at)
        {
            cut = true;
        }
        // A request that a covering deadline cut returns once the run's clock has reached that
        // deadline, never before, as natively, so the `within` it covers sees its deadline
        // passed (`spec/27 §5` rule 6, #1355). Under virtual time the clock is there already.
        if cut
            && sent == Err(Error::TimedOut)
            && let Some(at) = cover
        {
            self.run.clock.wait_until(at);
        }
        if cut && by_ever && sent == Err(Error::TimedOut) {
            self.timeout_event();
        }
        let response = sent.map_err(fail)?;
        let headers: Vec<Value<'p>> = response
            .headers
            .iter()
            .map(|(name, value)| {
                Value::data(
                    prelude_ctor("Header", "Header", 0),
                    vec![Value::text(name), Value::text(value)],
                )
            })
            .collect();
        Ok(Value::data(
            prelude_ctor("Response", "Response", 0),
            vec![
                Value::int(i64::from(response.status)),
                Value::List(Rc::new(headers)),
                Value::text(&response.body),
            ],
        ))
    }

    /// Makes the request `owned`, which holds a place of `spec/27 §2` rule 3, on a helper thread
    /// while this task parks (#1730), with the same settings, covering deadline, host stop and
    /// limits as on the run's thread, so its outcome is one the thread could have had. The task
    /// wakes once the helper has returned, gives the place back and joins the helper, so no
    /// helper outlives its task. Nothing interrupts the request but its deadline and, in the body
    /// of a `for ever`, a host stop: a cancellation of the task, or an abort of the run, leaves it
    /// running to its deadline, and the cancelled task stops at its next check point with its
    /// result dropped (#1811). A helper that cannot start leaves the request to the run's thread.
    /// O(log f) for the f requests in flight, beside the request and one thread's start.
    fn http_off_thread(
        &mut self,
        offload: &crate::tasks::Offload,
        owned: Owned,
        settings: una_http::client::Settings,
        cover: Option<u64>,
        stop: Option<Arc<RunStop>>,
    ) -> Result<Result<una_http::client::Response, una_http::client::Error>, Stop<'p>> {
        use std::sync::atomic::{AtomicBool, Ordering};
        let id = self.run.current();
        let done = Arc::new(AtomicBool::new(false));
        offload.flights.borrow_mut().insert(id);
        let owned = Arc::new(owned);
        let helper = {
            let (landed, done) = (Arc::clone(&offload.landed), Arc::clone(&done));
            let (owned, stop) = (Arc::clone(&owned), stop.clone());
            let governor = Arc::clone(&self.run.http_limits);
            std::thread::Builder::new()
                .name("una-http".to_owned())
                .spawn(move || {
                    // Rings on every way out, an unwind's included, so the task always wakes.
                    let _ring = Ring(landed, id, done);
                    send(&owned, &governor, &settings, stop.as_deref(), cover)
                })
        };
        let Ok(helper) = helper else {
            offload.flights.borrow_mut().remove(&id);
            self.run.give_place(offload);
            let limits = &self.run.http_limits;
            return Ok(send(&owned, limits, &settings, stop.as_deref(), cover));
        };
        // Only the request's landing wakes the task: the driver puts it back in the queue,
        // removes its flight and gives its place back. A helper that returned before the task
        // looked, as at a refused connection, rang with no park after it: the task takes its own
        // ring back (`settle`), so no later wake of the driver reaches it out of turn. No check
        // point runs between the helper's start and this look, so nothing else takes the ring.
        while !done.load(Ordering::SeqCst) {
            // A park is one of the task's switches: the tasks it holds run first (#1545).
            self.run.at_switch();
            if done.load(Ordering::SeqCst) {
                break;
            }
            if !self.run.park() {
                // INVARIANT: `offloads` is `Some` only while the driver serves the queue.
                return Err(fault("an `Http` request off the thread with no scheduler"));
            }
        }
        self.run.settle(offload, id);
        helper
            .join()
            .map_err(|_| fault("the helper of an `Http` request panicked"))
    }

    /// A host operation, within the grants (`spec/14 §3`).
    fn host(&mut self, capability: &str, op: &str, args: &[Value<'p>], span: Span) -> Flow<'p> {
        let text = |i: usize| match args.get(i) {
            Some(Value::Text(t)) => t.to_string(),
            _ => String::new(),
        };
        let file_error = |name: &str, index: usize| {
            Stop::Fail(Value::data(
                prelude_ctor("FileError", name, index),
                Vec::new(),
            ))
        };
        match (capability, op) {
            ("Console", "print") => {
                self.run.world.borrow_mut().print(&text(0));
                Ok(Value::Unit)
            }
            // Under virtual time, the run's virtual clock, from 0 at its start, instead of the
            // wall clock (ADR 0116 rule 19, `spec/26 §8`).
            ("Clock", "now") => Ok(Value::int(if self.run.clock.is_virtual() {
                i64::try_from(self.run.clock.now()).unwrap_or(i64::MAX)
            } else {
                self.run.world.borrow_mut().now()
            })),
            // A value that is not valid UTF-8 aborts the run: its content, not the name,
            // would decide the `Option`'s `public` tag (S-16, ADR 0086).
            ("Env", "get") => match self.run.world.borrow_mut().env_var(&text(0)) {
                Ok(Some(value)) => Ok(some(Value::text(&value))),
                Ok(None) => Ok(none()),
                Err(host::NotText) => Err(Stop::Abort(Abort::NotText {
                    span,
                    message: "the value of this environment variable is not valid UTF-8",
                })),
            },
            ("Files", "read" | "write") => {
                let writing = op == "write";
                let path = text(0);
                // The first grant of the access that does not deny the path decides.
                let mut target = host::Confined::Denied;
                for grant in self.grants {
                    if let Grant::Files {
                        prefix,
                        read,
                        write,
                    } = grant
                        && (if writing { *write } else { *read })
                    {
                        target = host::confine(&mut **self.run.world.borrow_mut(), prefix, &path);
                        if target != host::Confined::Denied {
                            break;
                        }
                    }
                }
                let done = match target {
                    host::Confined::Denied => return Err(file_error("Denied", 1)),
                    // #1459: an intermediate directory is confirmed absent, so the operation
                    // fails here and the world is given no path.
                    host::Confined::Absent(absence) => Err(absence.fault()),
                    host::Confined::Path(target) if writing => self
                        .run
                        .world
                        .borrow_mut()
                        .write_file(&target, &text(1))
                        .map(|()| Value::Unit),
                    host::Confined::Path(target) => {
                        let read = self.run.world.borrow_mut().read_file(&target);
                        read.map(|t| Value::text(&t))
                    }
                };
                done.map_err(|problem| match problem {
                    FileFault::NotFound => file_error("NotFound", 0),
                    FileFault::Io => file_error("Io", 2),
                    // Not a failure: the run aborts, since the content, not the path or the
                    // file system's shape, decides it (S-16, ADR 0086).
                    FileFault::NotText => Stop::Abort(Abort::NotText {
                        span,
                        message: "the content of this file is not valid UTF-8",
                    }),
                })
            }
            ("Http", "get" | "post" | "put" | "delete") => self.http(op, args, span),
            // ADR 0123 rules 1 to 4, `spec/31 §1`.
            ("HttpServer", _) => self.server(op, args, span),
            ("Keys", "hmac") => match self.keys.get(&text(0)) {
                Some(key) => Ok(Value::Hashed(Rc::from(host::hmac_sha256(
                    key,
                    text(1).as_bytes(),
                )))),
                None => Err(Stop::Fail(Value::data(
                    prelude_ctor("KeyError", "UnknownKey", 0),
                    Vec::new(),
                ))),
            },
            _ => Err(fault(format!("no host operation `{capability}.{op}`"))),
        }
    }

    /// The functions of the prelude (`spec/14 §1`).
    fn prelude(&mut self, name: &str, args: Vec<Value<'p>>, span: Span) -> Flow<'p> {
        if matches!(name, "append" | "set") {
            return update(name, args, span);
        }
        // Dense values (`spec/29 §17`, ADR 0120).
        if dense::is_dense(name) {
            return self.dense_call(name, args, span);
        }
        let bad = || fault(format!("`{name}` applied to values of the wrong types"));
        let int = |i: usize| match args.get(i) {
            Some(Value::Int(n)) => Ok(n.clone()),
            _ => Err(bad()),
        };
        let text = |i: usize| match args.get(i) {
            Some(Value::Text(t)) => Ok(t.clone()),
            _ => Err(bad()),
        };
        let list = |i: usize| match args.get(i) {
            Some(Value::List(items)) => Ok(items.clone()),
            _ => Err(bad()),
        };
        let arg = |i: usize| args.get(i).cloned().ok_or_else(bad);
        Ok(match name {
            "abs" => Value::Int(BigInt::from_biguint(
                Sign::Plus,
                int(0)?.magnitude().clone(),
            )),
            // Generic over `Ord` in edition 0.2 (`spec/20 §6`): the built-in order of the
            // values, the first argument when they are equal.
            "min" => {
                let (a, b) = (arg(0)?, arg(1)?);
                if compare(&b, &a) == std::cmp::Ordering::Less {
                    b
                } else {
                    a
                }
            }
            "max" => {
                let (a, b) = (arg(0)?, arg(1)?);
                if compare(&b, &a) == std::cmp::Ordering::Greater {
                    b
                } else {
                    a
                }
            }
            "empty_map" => Value::Map(Rc::default()),
            // JSON (`spec/23`), by the parser and writer that the native runtime shares.
            // Both check the memory budget as they go, and stop at this call over it.
            "parse_json" => {
                let source = text(0)?;
                let (held, baseline) = (self.held, self.baseline);
                let budget = self.max_memory.saturating_mul(1024 * 1024);
                let mut within = || held().saturating_sub(baseline) <= budget;
                let over = Stop::Abort(Abort::Memory {
                    span,
                    limit: self.max_memory,
                });
                match una_core::json::parse_within(&source, &mut within) {
                    Ok(tree) => match crate::json::value(tree, &mut within) {
                        Some(value) => value,
                        None => return Err(over),
                    },
                    Err(Halt::Invalid(error)) => {
                        return Err(Stop::Fail(crate::json::error(error)));
                    }
                    Err(Halt::OverBudget) => return Err(over),
                }
            }
            "to_json" => {
                let (held, baseline) = (self.held, self.baseline);
                let budget = self.max_memory.saturating_mul(1024 * 1024);
                let mut within = || held().saturating_sub(baseline) <= budget;
                let over = Stop::Abort(Abort::Memory {
                    span,
                    limit: self.max_memory,
                });
                let room = |bytes| self.fits(bytes);
                let tree = match crate::json::tree(&arg(0)?, &mut within, &room) {
                    Ok(tree) => tree,
                    Err(crate::json::NoTree::NotJson) => return Err(bad()),
                    Err(crate::json::NoTree::OverBudget) => return Err(over),
                };
                match una_core::json::write_within(&tree, &mut within) {
                    Some(text) => Value::text(&text),
                    None => return Err(over),
                }
            }
            "quote_json" => Value::text(&una_core::json::quote(&text(0)?)),
            "is_json_number" => Value::Bool(una_core::json::is_json_number(&text(0)?)),
            // Ascending and stable (`spec/20 §6` rule 2).
            "sort" => {
                let mut items = list(0)?.as_ref().clone();
                items.sort_by(compare);
                Value::List(Rc::new(items))
            }
            "is_digits" => Value::Bool(text(0)?.chars().all(|c| c.is_ascii_digit())),
            "starts_with" => Value::Bool(text(0)?.starts_with(text(1)?.as_str())),
            "trim" => Value::text(text(0)?.trim()),
            // The text functions of edition 0.2 (`spec/22 §2`).
            "split" => {
                let (t, on) = (text(0)?, text(1)?);
                Value::List(Rc::new(t.split(on.as_str()).map(Value::text).collect()))
            }
            "join" => {
                let parts = list(0)?;
                let separator = text(1)?;
                let mut out = String::new();
                for (i, part) in parts.iter().enumerate() {
                    if i > 0 {
                        out.push_str(&separator);
                    }
                    match part {
                        Value::Text(t) => out.push_str(t),
                        _ => return Err(bad()),
                    }
                }
                Value::text(&out)
            }
            "ends_with" => Value::Bool(text(0)?.ends_with(text(1)?.as_str())),
            "contains_text" => Value::Bool(search(&text(0)?, &text(1)?).is_some()),
            "find" => {
                let t = text(0)?;
                match search(&t, &text(1)?) {
                    Some(byte) => some(Value::int(
                        i64::try_from(t.scalars_before(byte)).unwrap_or(i64::MAX),
                    )),
                    None => none(),
                }
            }
            "replace" => {
                let (t, pattern, by) = (text(0)?, text(1)?, text(2)?);
                // Its `requires` excludes an empty pattern: at the call, as native code aborts
                // (#693, `spec/22 §2` rule 3).
                if pattern.is_empty() {
                    return Err(precondition(
                        "`replace` with an empty pattern, which its precondition excludes",
                        span,
                    ));
                }
                Value::text(&t.replace(pattern.as_str(), &by))
            }
            "slice" => {
                let t = text(0)?;
                let (start, end) = (int(1)?, int(2)?);
                let position = |n: &BigInt| usize::try_from(n).ok();
                // Its `requires` keeps `0 <= start <= end <= t.len`: outside it, at the call, as
                // native code aborts (#693, `spec/22 §2` rule 4).
                let (Some(start), Some(end)) = (position(&start), position(&end)) else {
                    return Err(precondition(SLICE_OUTSIDE, span));
                };
                if start > end || end > t.scalars() {
                    return Err(precondition(SLICE_OUTSIDE, span));
                }
                // O(result) once the text's index is built (`spec/22 §6` rule 1).
                Value::text(t.slice(start, end))
            }
            "to_upper" => Value::text(&text(0)?.to_ascii_uppercase()),
            "to_lower" => Value::text(&text(0)?.to_ascii_lowercase()),
            "chars" => Value::List(Rc::new(
                text(0)?
                    .chars()
                    .map(|c| Value::text(c.encode_utf8(&mut [0; 4])))
                    .collect(),
            )),
            "parse_int" => return parse_int(&text(0)?),
            // The functions of `F64` (`spec/29`, ADR 0114 rule 3.9).
            "nan" => Value::F64(f64::from_bits(una_core::float::CANONICAL_NAN)),
            "infinity" => Value::F64(f64::INFINITY),
            "f64_from_bits" => {
                let bits = u64::try_from(&int(0)?).map_err(|_| bad())?;
                Value::F64(una_core::float::from_bits(bits))
            }
            "parse_f64" => return parse_f64(&text(0)?),
            // The functions of `Decimal` (`spec/29 §16`, ADR 0120 rules 16 and 17): the scale
            // is the `scale:` literal, an `Int` operand.
            "parse_decimal" => {
                let scale = u8::try_from(&int(1)?).map_err(|_| bad())?;
                return parse_decimal(&text(0)?, scale);
            }
            // Elaboration writes `json_decimal(x)` as an operation that carries the scale of
            // `x` (`spec/29 §16`): a call of the prelude's stub is an internal error.
            "json_decimal" => {
                return Err(fault(
                    "`json_decimal` reached the prelude's stub, without the scale of its argument",
                ));
            }
            "as_decimal" => {
                let scale = u8::try_from(&int(1)?).map_err(|_| bad())?;
                match arg(0)? {
                    Value::Data(data) => match (data.ctor.name.as_str(), data.fields.first()) {
                        ("Number", Some(Value::Int(n))) => {
                            some(Value::Decimal(una_core::decimal::widen(n, 0, scale), scale))
                        }
                        ("Decimal", Some(Value::Text(t))) => {
                            match una_core::decimal::from_json_number(t, scale) {
                                Some(c) => some(Value::Decimal(c, scale)),
                                None => none(),
                            }
                        }
                        _ => none(),
                    },
                    _ => return Err(bad()),
                }
            }
            // The functions of `F32` (ADR 0120 rule 5).
            "f32_from_bits" => {
                let bits = u32::try_from(&int(0)?).map_err(|_| bad())?;
                Value::F32(una_core::float32::from_bits(bits))
            }
            "parse_f32" => return parse_f32(&text(0)?),
            "as_f64" => match arg(0)? {
                Value::Data(data) => match (data.ctor.name.as_str(), data.fields.first()) {
                    ("Number", Some(Value::Int(n))) => {
                        some(Value::F64(una_core::float::from_int(n)))
                    }
                    ("Decimal", Some(Value::Text(t))) => match una_core::float::parse(t) {
                        Ok(x) => some(Value::F64(x)),
                        Err(_) => return Err(bad()),
                    },
                    _ => none(),
                },
                _ => return Err(bad()),
            },
            "json_number" => match arg(0)? {
                Value::F64(x) if x.is_finite() => {
                    some(crate::json::decimal(&una_core::float::render(x)))
                }
                Value::F64(_) => none(),
                _ => return Err(bad()),
            },
            "trim_start" => Value::text(text(0)?.trim_start()),
            "trim_end" => Value::text(text(0)?.trim_end()),
            "bytes" => Value::List(Rc::new(
                text(0)?.bytes().map(|b| Value::int(i64::from(b))).collect(),
            )),
            "to_int" => {
                let t = text(0)?;
                let digits = t.strip_prefix('-').unwrap_or(&t);
                // Checked in O(b) before the O(d²) conversion, which only a text of at most
                // MAX_INT_DIGITS digits reaches (`spec/14 §1`, #914).
                let valid = !digits.is_empty()
                    && digits.len() <= una_core::MAX_INT_DIGITS
                    && digits.bytes().all(|b| b.is_ascii_digit());
                if valid {
                    t.parse::<BigInt>()
                        .map_or_else(|_| none(), |n| some(Value::Int(n)))
                } else {
                    none()
                }
            }
            "is_empty" => Value::Bool(list(0)?.is_empty()),
            "first" => list(0)?.first().cloned().map_or_else(none, some),
            "at" => {
                let items = list(0)?;
                let index = int(1)?;
                usize::try_from(&index)
                    .ok()
                    .and_then(|i| items.get(i).cloned())
                    .map_or_else(none, some)
            }
            "contains" => {
                let item = arg(1)?;
                let mut found = false;
                for v in list(0)?.iter() {
                    if self.equal_at(v, &item, span)? {
                        found = true;
                        break;
                    }
                }
                Value::Bool(found)
            }
            "map" => {
                let (items, each) = (list(0)?, arg(1)?);
                let mut out = Vec::with_capacity(items.len());
                for item in items.iter() {
                    out.push(self.apply(&each, vec![item.clone()], span)?);
                }
                Value::List(Rc::new(out))
            }
            "filter" => {
                let (items, keep) = (list(0)?, arg(1)?);
                let mut out = Vec::new();
                for item in items.iter() {
                    if matches!(
                        self.apply(&keep, vec![item.clone()], span)?,
                        Value::Bool(true)
                    ) {
                        out.push(item.clone());
                    }
                }
                Value::List(Rc::new(out))
            }
            "first_where" => {
                let (items, keep) = (list(0)?, arg(1)?);
                let mut found = none();
                for item in items.iter() {
                    if matches!(
                        self.apply(&keep, vec![item.clone()], span)?,
                        Value::Bool(true)
                    ) {
                        found = some(item.clone());
                        break;
                    }
                }
                found
            }
            "fold" => {
                let (items, step) = (list(0)?, arg(2)?);
                let mut acc = arg(1)?;
                for item in items.iter() {
                    acc = self.apply(&step, vec![acc, item.clone()], span)?;
                }
                acc
            }
            "sum" => {
                let mut total = BigInt::from(0);
                for item in list(0)?.iter() {
                    match item {
                        Value::Int(n) => total += n,
                        _ => return Err(bad()),
                    }
                }
                Value::Int(total)
            }
            "is_some" | "is_ok" => match arg(0)? {
                Value::Data(data) => Value::Bool(data.ctor.index == 0),
                _ => return Err(bad()),
            },
            "or_else" => match arg(0)? {
                Value::Data(data) if data.ctor.index == 0 => {
                    data.fields.first().cloned().ok_or_else(bad)?
                }
                _ => arg(1)?,
            },
            _ => return Err(fault(format!("no prelude function `{name}`"))),
        })
    }
}

/// The stack kept free below the evaluation budget, for the recursion between two calls: a
/// function body's nested expressions, and rendering values.
const STACK_RESERVE: usize = 8 * 1024 * 1024;

/// The address of `local`, a variable of the caller's frame: a measure of the stack in use.
fn stack_position<T>(local: &T) -> usize {
    std::ptr::from_ref(local).addr()
}

/// An operation call: what its event records.
struct Request<'a> {
    cap: &'a str,
    capability: &'a str,
    op: &'a str,
    labels: &'a [Label],
    /// The label of the binder of its result, which redacts its value.
    result: Label,
    /// The label of its outcome: Core's, joined with the binder's (S-45, #1552).
    outcome: Label,
    span: Span,
}

/// The variant of a failure.
pub(crate) fn variant(failure: &Value<'_>) -> String {
    match failure {
        Value::Data(data) => data.ctor.name.clone(),
        _ => "?".to_owned(),
    }
}

/// `H|dom(K)`: the instances of `caps` bound to the names of `uses`.
fn restrict<'p>(caps: &Caps<'p>, uses: &'p [Use]) -> Caps<'p> {
    Rc::new(
        uses.iter()
            .filter_map(|u| {
                caps.get(u.cap.as_str())
                    .map(|instance| (u.cap.as_str(), instance.clone()))
            })
            .collect(),
    )
}

/// Euclidean division and remainder: `a = b * q + r` with `0 <= r < |b|` (ADR 0004).
fn div_euclid(a: &BigInt, b: &BigInt) -> (BigInt, BigInt) {
    let q = a / b;
    let r = a % b;
    if r.sign() == Sign::Minus {
        if b.sign() == Sign::Plus {
            (q - 1, r + b)
        } else {
            (q + 1, r - b)
        }
    } else {
        (q, r)
    }
}

/// Whether `value` lies in the range of `int`.
fn in_range(value: &BigInt, int: IntType) -> bool {
    BigInt::from(int.least()) <= *value && *value <= BigInt::from(int.greatest())
}

/// `value` reduced into the range of `int` modulo `2^width` (`spec/18 §5`).
fn wrap(value: &BigInt, int: IntType) -> BigInt {
    let modulus = BigInt::from(1) << int.bits();
    let (_, mut r) = div_euclid(value, &modulus);
    if r > BigInt::from(int.greatest()) {
        r -= modulus;
    }
    r
}

/// `value` clamped to the range of `int` (`spec/18 §5`).
fn saturate(value: BigInt, int: IntType) -> BigInt {
    let (least, greatest) = (BigInt::from(int.least()), BigInt::from(int.greatest()));
    if value < least {
        least
    } else if value > greatest {
        greatest
    } else {
        value
    }
}

/// A method of a fixed-width type over its operands' values (`spec/18 §5`).
fn fixed_apply<'p>(op: FixedOp, int: IntType, args: &[Value<'p>]) -> Result<BigInt, Stop<'p>> {
    let int_at = |i: usize| match args.get(i) {
        Some(Value::Int(n)) => Ok(n.clone()),
        _ => Err(fault(format!(
            "`{}` applied to a value that is not an integer",
            op.as_str()
        ))),
    };
    let a = int_at(0)?;
    let b = if op.binary() {
        int_at(1)?
    } else {
        BigInt::from(0)
    };
    fixed_method(op, int, &a, &b).ok_or_else(|| {
        fault(format!(
            "`{}` by {b}, outside `0` to `{}`, which a precondition excludes",
            op.as_str(),
            int.bits() - 1
        ))
    })
}

/// Method `op` of the fixed-width type `int` on `a`, with the argument `b` (`spec/18 §5`), or
/// `None` for a shift by an amount outside the width, which a precondition excludes. Bit
/// operations act on the two's-complement bits of the width; `shift_right` divides by `2^n`
/// rounding toward negative infinity, so it is arithmetic on signed types. The native runtime
/// computes the methods with this function too.
pub fn fixed_method(op: FixedOp, int: IntType, a: &BigInt, b: &BigInt) -> Option<BigInt> {
    if op.shift() && (b.sign() == Sign::Minus || *b >= BigInt::from(int.bits())) {
        return None;
    }
    let (a, b) = (a.clone(), b.clone());
    Some(match op {
        FixedOp::WrappingAdd => wrap(&(a + b), int),
        FixedOp::WrappingSub => wrap(&(a - b), int),
        FixedOp::WrappingMul => wrap(&(a * b), int),
        FixedOp::SaturatingAdd => saturate(a + b, int),
        FixedOp::SaturatingSub => saturate(a - b, int),
        FixedOp::SaturatingMul => saturate(a * b, int),
        FixedOp::BitAnd => wrap(&(wrap_bits(&a, int) & wrap_bits(&b, int)), int),
        FixedOp::BitOr => wrap(&(wrap_bits(&a, int) | wrap_bits(&b, int)), int),
        FixedOp::BitXor => wrap(&(wrap_bits(&a, int) ^ wrap_bits(&b, int)), int),
        FixedOp::ShiftRight => {
            let divisor = BigInt::from(1) << usize::try_from(b).unwrap_or(0);
            let (q, _) = div_euclid(&a, &divisor);
            q
        }
        FixedOp::WrappingShiftLeft => wrap(&(a << usize::try_from(b).unwrap_or(0)), int),
        FixedOp::BitNot => wrap(&!wrap_bits(&a, int), int),
    })
}

/// The bits of `value` in the width of `int`, as a non-negative integer below `2^width`.
fn wrap_bits(value: &BigInt, int: IntType) -> BigInt {
    let modulus = BigInt::from(1) << int.bits();
    div_euclid(value, &modulus).1
}

fn prim_apply<'p>(prim: Prim, args: &[Value<'p>]) -> Flow<'p> {
    let int = |i: usize| match args.get(i) {
        Some(Value::Int(n)) => Ok(n),
        _ => Err(fault(format!(
            "`{}` applied to a value that is not an `Int`",
            prim.as_str()
        ))),
    };
    let arg = |i: usize| {
        args.get(i)
            .ok_or_else(|| fault(format!("`{}` without its operands", prim.as_str())))
    };
    Ok(match prim {
        Prim::Add => Value::Int(int(0)? + int(1)?),
        Prim::Sub => Value::Int(int(0)? - int(1)?),
        Prim::Mul => Value::Int(int(0)? * int(1)?),
        Prim::Div | Prim::Mod => {
            let (a, b) = (int(0)?, int(1)?);
            if b.sign() == Sign::NoSign {
                // Tier 1 proves divisors nonzero; until it runs, zero aborts.
                return Err(fault(format!(
                    "`{}` by zero, which a proof obligation excludes once tier 1 verifies it",
                    prim.as_str()
                )));
            }
            let (q, r) = div_euclid(a, b);
            Value::Int(if prim == Prim::Div { q } else { r })
        }
        Prim::Neg => Value::Int(-int(0)?.clone()),
        Prim::Not => match arg(0)? {
            Value::Bool(b) => Value::Bool(!b),
            _ => return Err(fault("`not` applied to a value that is not a `Bool`")),
        },
        // Compared at their call, within the memory budget (`Machine::equal_at`).
        Prim::Eq | Prim::Ne => {
            return Err(fault(format!("`{}` outside its call", prim.as_str())));
        }
        Prim::Lt => Value::Bool(compare(arg(0)?, arg(1)?).is_lt()),
        Prim::Le => Value::Bool(compare(arg(0)?, arg(1)?).is_le()),
        Prim::Gt => Value::Bool(compare(arg(0)?, arg(1)?).is_gt()),
        Prim::Ge => Value::Bool(compare(arg(0)?, arg(1)?).is_ge()),
        Prim::Len => match arg(0)? {
            Value::Text(t) => Value::Int(BigInt::from(t.scalars())),
            Value::List(items) => Value::Int(BigInt::from(items.len())),
            Value::Map(entries) => Value::Int(BigInt::from(entries.len())),
            Value::Seq(items) => Value::Int(BigInt::from(items.len())),
            Value::Set(items) => Value::Int(BigInt::from(items.len())),
            Value::HashMap(table) | Value::HashSet(table) => {
                Value::Int(BigInt::from(table.entries.len()))
            }
            Value::Priority(heap) => Value::Int(BigInt::from(heap.entries.len())),
            // A bit set's count of members, kept with its words (`spec/28 §9`).
            Value::BitSet(bits) => Value::Int(BigInt::from(bits.count)),
            // An array's length (`spec/29 §17`).
            Value::Dense(dense) => {
                Value::Int(BigInt::from(dense.shape.first().copied().unwrap_or(0)))
            }
            _ => {
                return Err(fault(
                    "`len` of a value that is not a text, list, map, sequence, set, hash table, priority queue, bit set or array",
                ));
            }
        },
        Prim::Concat => {
            let mut out = String::new();
            for value in args {
                match value {
                    Value::Text(t) => out.push_str(t),
                    _ => return Err(fault("`concat` of a value that is not a `Text`")),
                }
            }
            Value::text(&out)
        }
        Prim::Convert(target) => {
            let value = int(0)?;
            if let Some(int) = target
                && !in_range(value, int)
            {
                return Err(fault(format!(
                    "`{}` of a value outside `{}`, which an `overflow` obligation excludes",
                    prim.as_str(),
                    int.as_str()
                )));
            }
            Value::Int(value.clone())
        }
        Prim::Fixed(op, int) => Value::Int(fixed_apply(op, int, args)?),
        // A copy of the map when another name holds it; a binding at its last use gives it up
        // first, in `Machine::eval`.
        Prim::Map(op) => return map_apply(op, args.to_vec()),
        Prim::Seq(op) => return seqs::seq_apply(op, args.to_vec()),
        Prim::Set(op) => return sets::set_apply(op, args.to_vec()),
        // `Machine::rhs` applies them, with the memory budget of their key comparisons.
        Prim::HashMap(_) | Prim::HashSet(_) => {
            return Err(fault("a hash table operation outside the machine"));
        }
        Prim::Priority(op) => return priority::priority_apply(op, args.to_vec()),
        // Applied at its call, with its span, in `Machine::rhs`.
        Prim::BitSet(op) => return bitsets::bit_set_apply(op, args.to_vec(), None),
        Prim::Float(op) => return float_apply(op, args),
        Prim::Decimal(op) => return decimal_apply(op, args),
        // The shape of a matrix, a tensor or an `Lu[T]` (`spec/29 §17`).
        Prim::Shape(fun) => match arg(0)? {
            Value::Dense(dense) => {
                // Exact, as `len` is: a dimension is the one the program gave.
                let value = match fun {
                    una_core::ShapeFun::Rows => dense.shape.first().copied().unwrap_or(0),
                    // An `Lu[T]` of size n holds n + 1 rows of n (`DenseKind::Lu`).
                    una_core::ShapeFun::Columns | una_core::ShapeFun::Size => {
                        dense.shape.get(1).copied().unwrap_or(0)
                    }
                    una_core::ShapeFun::Rank => dense.shape.len(),
                    una_core::ShapeFun::Count => dense.count(),
                };
                Value::Int(BigInt::from(value))
            }
            _ => return Err(fault("a shape of a value that is not a dense value")),
        },
        Prim::Float32(op) => return float32_apply(op, args),
        Prim::ToText => match arg(0)? {
            // In O(M(d) log d) for d digits, of any size (`spec/11 §7`, #1223).
            Value::Int(n) => Value::text(&n.to_string()),
            Value::Bool(b) => Value::text(&b.to_string()),
            Value::Text(t) => Value::Text(t.clone()),
            _ => {
                return Err(fault(
                    "`to_text` of a value other than an `Int`, `Bool` or `Text`",
                ));
            }
        },
    })
}

/// A map's built-in method (`spec/21 §2`), in O(log n) for a lookup or an update of a map it
/// holds the only reference to, and with an O(n) copy first otherwise (`§5` rule 2).
fn map_apply<'p>(op: MapOp, args: Vec<Value<'p>>) -> Flow<'p> {
    let bad = || {
        fault(format!(
            "map `{}` applied to values of the wrong types",
            op.as_str()
        ))
    };
    let mut args = args.into_iter();
    let Some(Value::Map(mut entries)) = args.next() else {
        return Err(bad());
    };
    let key = match op {
        MapOp::Has | MapOp::Get | MapOp::With | MapOp::Without => {
            let value = args.next().ok_or_else(bad)?;
            Some((Key::of(&value).ok_or_else(bad)?, value))
        }
        MapOp::Keys | MapOp::Values | MapOp::Entries => None,
    };
    Ok(match (op, key) {
        (MapOp::Has, Some((key, _))) => Value::Bool(entries.contains_key(&key)),
        (MapOp::Get, Some((key, _))) => match entries.get(&key) {
            Some((_, value)) => some(value.clone()),
            None => none(),
        },
        (MapOp::With, Some((key, written))) => {
            let value = args.next().ok_or_else(bad)?;
            Rc::make_mut(&mut entries).insert(key, (written, value));
            Value::Map(entries)
        }
        (MapOp::Without, Some((key, _))) => {
            if entries.contains_key(&key) {
                Rc::make_mut(&mut entries).remove(&key);
            }
            Value::Map(entries)
        }
        (MapOp::Keys, None) => Value::List(Rc::new(
            entries.values().map(|(key, _)| key.clone()).collect(),
        )),
        (MapOp::Values, None) => Value::List(Rc::new(
            entries.values().map(|(_, value)| value.clone()).collect(),
        )),
        (MapOp::Entries, None) => Value::List(Rc::new(
            entries
                .values()
                .map(|(key, value)| {
                    Value::data(
                        prelude_ctor("Entry", "Entry", 0),
                        vec![key.clone(), value.clone()],
                    )
                })
                .collect(),
        )),
        _ => return Err(bad()),
    })
}

/// The first occurrence of `part` in `t`, as a byte offset: the one search that `find` and
/// `contains_text` share, so that one is `Some` exactly when the other holds. Rust's
/// search is linear in the worst case (`spec/22 §2` rule 8).
fn search(t: &str, part: &str) -> Option<usize> {
    t.find(part)
}

/// `parse_int(t)` (`spec/22 §4`): an optional `-` and one or more ASCII digits, or a failure
/// that says why: `Empty`, `NoDigits` for `-` alone, `NotADigit(at: i)` at the first
/// position that is neither, or `TooLong` for more than `MAX_INT_DIGITS` digits, found in O(b)
/// before the O(d²) conversion (#914).
fn parse_int<'p>(t: &str) -> Flow<'p> {
    let failure = |name: &str, index: usize, fields: Vec<Value<'p>>| {
        Err(Stop::Fail(Value::data(
            prelude_ctor("ParseError", name, index),
            fields,
        )))
    };
    if t.is_empty() {
        return failure("Empty", 0, Vec::new());
    }
    let digits = t.strip_prefix('-').unwrap_or(t);
    let signed = digits.len() < t.len();
    if let Some((at, _)) = digits
        .chars()
        .enumerate()
        .find(|(_, c)| !c.is_ascii_digit())
    {
        let at = at + usize::from(signed);
        return failure(
            "NotADigit",
            1,
            vec![Value::int(i64::try_from(at).unwrap_or(i64::MAX))],
        );
    }
    if digits.is_empty() {
        return failure("NoDigits", 2, Vec::new());
    }
    if digits.len() > una_core::MAX_INT_DIGITS {
        return failure("TooLong", 3, Vec::new());
    }
    match t.parse::<BigInt>() {
        Ok(n) => Ok(Value::Int(n)),
        Err(_) => failure("NoDigits", 2, Vec::new()),
    }
}

/// `parse_decimal(t, scale: s)` (`spec/29 §16`, ADR 0120 rule 16): a failure says why, as
/// `parse_int`'s does, `TooLong` past the shared digit cap.
fn parse_decimal<'p>(t: &str, scale: u8) -> Flow<'p> {
    use una_core::decimal::ParseFailure;
    let failure = |name: &str, index: usize, fields: Vec<Value<'p>>| {
        Err(Stop::Fail(Value::data(
            prelude_ctor("ParseError", name, index),
            fields,
        )))
    };
    match una_core::decimal::parse(t, scale) {
        Ok(c) => Ok(Value::Decimal(c, scale)),
        Err(ParseFailure::Empty) => failure("Empty", 0, Vec::new()),
        Err(ParseFailure::NoDigits) => failure("NoDigits", 2, Vec::new()),
        Err(ParseFailure::NotADigit(at)) => failure(
            "NotADigit",
            1,
            vec![Value::int(i64::try_from(at).unwrap_or(i64::MAX))],
        ),
        Err(ParseFailure::TooLong) => failure("TooLong", 3, Vec::new()),
    }
}

/// An operation of `Decimal[s]` (`spec/29 §16`, ADR 0120 rules 13 to 17), exact, with the
/// static scales of its operands; each rounding is the `Rounding` operand's mode.
fn decimal_apply<'p>(op: DecimalOp, args: &[Value<'p>]) -> Flow<'p> {
    use una_core::decimal as d;
    let bad = || {
        fault(format!(
            "`{}` applied to values of the wrong types",
            op.as_str()
        ))
    };
    let coefficient = |i: usize| match args.get(i) {
        Some(Value::Decimal(c, _)) => Ok(c),
        _ => Err(bad()),
    };
    let rounding = |i: usize| match args.get(i) {
        Some(Value::Data(data)) => d::Rounding::from_index(data.ctor.index).ok_or_else(bad),
        _ => Err(bad()),
    };
    let compare = |a: u8, b: u8| -> Result<std::cmp::Ordering, Stop<'p>> {
        let m = a.max(b);
        Ok(d::widen(coefficient(0)?, a, m).cmp(&d::widen(coefficient(1)?, b, m)))
    };
    let division_by_zero =
        || fault("a decimal divided by zero, which its `division` obligation excludes");
    Ok(match op {
        DecimalOp::Add(a, b) | DecimalOp::Sub(a, b) => {
            let m = a.max(b);
            let (x, y) = (
                d::widen(coefficient(0)?, a, m),
                d::widen(coefficient(1)?, b, m),
            );
            let c = if matches!(op, DecimalOp::Add(..)) {
                x + y
            } else {
                x - y
            };
            Value::Decimal(c, m)
        }
        DecimalOp::Mul => match (args.first(), args.get(1)) {
            (Some(Value::Decimal(x, a)), Some(Value::Decimal(y, b))) => {
                Value::Decimal(x * y, a.saturating_add(*b))
            }
            _ => return Err(bad()),
        },
        DecimalOp::Neg => match args.first() {
            Some(Value::Decimal(x, a)) => Value::Decimal(-x, *a),
            _ => return Err(bad()),
        },
        DecimalOp::Abs => match args.first() {
            Some(Value::Decimal(x, a)) => Value::Decimal(
                BigInt::from_biguint(una_core::bigint::Sign::Plus, x.magnitude().clone()),
                *a,
            ),
            _ => return Err(bad()),
        },
        DecimalOp::Eq(a, b) => Value::Bool(compare(a, b)?.is_eq()),
        DecimalOp::Ne(a, b) => Value::Bool(compare(a, b)?.is_ne()),
        DecimalOp::Lt(a, b) => Value::Bool(compare(a, b)?.is_lt()),
        DecimalOp::Le(a, b) => Value::Bool(compare(a, b)?.is_le()),
        DecimalOp::Gt(a, b) => Value::Bool(compare(a, b)?.is_gt()),
        DecimalOp::Ge(a, b) => Value::Bool(compare(a, b)?.is_ge()),
        DecimalOp::Widen(a, s) => Value::Decimal(d::widen(coefficient(0)?, a, s), s),
        DecimalOp::Rescale(a, s) => {
            Value::Decimal(d::rescale(coefficient(0)?, a, s, rounding(1)?), s)
        }
        DecimalOp::RescaleExact(a, s) => {
            let c = coefficient(0)?;
            if !d::rescales_exactly(c, a, s) {
                return Err(fault(
                    "`rescale_exact` dropped a nonzero digit, which its precondition excludes",
                ));
            }
            Value::Decimal(d::rescale(c, a, s, d::Rounding::TowardZero), s)
        }
        DecimalOp::Divide(a, b, s) => Value::Decimal(
            d::divide(coefficient(0)?, a, coefficient(1)?, b, s, rounding(2)?)
                .ok_or_else(division_by_zero)?,
            s,
        ),
        DecimalOp::Remainder(a, b, s) => Value::Decimal(
            d::remainder(coefficient(0)?, a, coefficient(1)?, b, s).ok_or_else(division_by_zero)?,
            d::remainder_scale(a, b, s),
        ),
        DecimalOp::ToInt(a) => Value::Int(d::rescale(coefficient(0)?, a, 0, rounding(1)?)),
        DecimalOp::Coefficient => Value::Int(coefficient(0)?.clone()),
        DecimalOp::FromInt => match args.first() {
            Some(Value::Int(n)) => Value::Decimal(n.clone(), 0),
            _ => return Err(bad()),
        },
        DecimalOp::FromF64(s) => match args.first() {
            Some(Value::F64(x)) => match d::from_f64_bits(x.to_bits(), s, rounding(1)?) {
                Some(c) => some(Value::Decimal(c, s)),
                None => none(),
            },
            _ => return Err(bad()),
        },
        DecimalOp::ToF64(a) => Value::F64(d::to_f64(coefficient(0)?, a)),
        DecimalOp::ToText(a) => Value::text(&d::render(coefficient(0)?, a)),
        DecimalOp::ToJson(a) => {
            let c = coefficient(0)?;
            if a == 0 {
                crate::json::number(c.clone())
            } else {
                crate::json::decimal(&d::render(c, a))
            }
        }
        // Calls of the prelude, never primitives.
        DecimalOp::Parse(_) | DecimalOp::FromJson(_) => return Err(bad()),
    })
}

/// `parse_f64(t)` (`spec/29`, ADR 0114 rule 7.3): a failure says why, as `parse_int`'s does.
fn parse_f64<'p>(t: &str) -> Flow<'p> {
    use una_core::float::ParseFailure;
    let failure = |name: &str, index: usize, fields: Vec<Value<'p>>| {
        Err(Stop::Fail(Value::data(
            prelude_ctor("ParseError", name, index),
            fields,
        )))
    };
    match una_core::float::parse(t) {
        Ok(x) => Ok(Value::F64(x)),
        Err(ParseFailure::Empty) => failure("Empty", 0, Vec::new()),
        Err(ParseFailure::NoDigits) => failure("NoDigits", 2, Vec::new()),
        Err(ParseFailure::NotADigit(at)) => failure(
            "NotADigit",
            1,
            vec![Value::int(i64::try_from(at).unwrap_or(i64::MAX))],
        ),
    }
}

/// An operation of `F64` (`spec/29`, ADR 0114 rules 3 to 5), each O(1), each `F64` result
/// canonical (rule 9.8).
fn float_apply<'p>(op: FloatOp, args: &[Value<'p>]) -> Flow<'p> {
    use una_core::float as f;
    let x = |i: usize| match args.get(i) {
        Some(Value::F64(x)) => Ok(*x),
        _ => Err(fault(format!(
            "`{}` applied to a value that is not an `F64`",
            op.as_str()
        ))),
    };
    Ok(match op {
        FloatOp::Add => Value::F64(f::add(x(0)?, x(1)?)),
        FloatOp::Sub => Value::F64(f::sub(x(0)?, x(1)?)),
        FloatOp::Mul => Value::F64(f::mul(x(0)?, x(1)?)),
        FloatOp::Div => Value::F64(f::div(x(0)?, x(1)?)),
        FloatOp::Neg => Value::F64(f::neg(x(0)?)),
        FloatOp::Eq => Value::Bool(f::equal(x(0)?, x(1)?)),
        FloatOp::Ne => Value::Bool(!f::equal(x(0)?, x(1)?)),
        FloatOp::Lt => Value::Bool(f::total_cmp(x(0)?, x(1)?).is_lt()),
        FloatOp::Le => Value::Bool(f::total_cmp(x(0)?, x(1)?).is_le()),
        FloatOp::Gt => Value::Bool(f::total_cmp(x(0)?, x(1)?).is_gt()),
        FloatOp::Ge => Value::Bool(f::total_cmp(x(0)?, x(1)?).is_ge()),
        FloatOp::Abs => Value::F64(f::abs(x(0)?)),
        FloatOp::Sqrt => Value::F64(f::sqrt(x(0)?)),
        FloatOp::Fma => Value::F64(f::fma(x(0)?, x(1)?, x(2)?)),
        FloatOp::Rem => Value::F64(f::rem(x(0)?, x(1)?)),
        FloatOp::Floor => Value::F64(f::floor(x(0)?)),
        FloatOp::Ceil => Value::F64(f::ceil(x(0)?)),
        FloatOp::Trunc => Value::F64(f::trunc(x(0)?)),
        FloatOp::RoundHalfEven => Value::F64(f::round_half_even(x(0)?)),
        FloatOp::RoundHalfAway => Value::F64(f::round_half_away(x(0)?)),
        FloatOp::CopySign => Value::F64(f::copy_sign(x(0)?, x(1)?)),
        FloatOp::IeeeMin => Value::F64(f::ieee_min(x(0)?, x(1)?)),
        FloatOp::IeeeMax => Value::F64(f::ieee_max(x(0)?, x(1)?)),
        FloatOp::Clamp => Value::F64(f::clamp(x(0)?, x(1)?, x(2)?)),
        FloatOp::IeeeEq => Value::Bool(f::ieee_compare(f::IeeeCompare::Eq, x(0)?, x(1)?)),
        FloatOp::IeeeNe => Value::Bool(f::ieee_compare(f::IeeeCompare::Ne, x(0)?, x(1)?)),
        FloatOp::IeeeLt => Value::Bool(f::ieee_compare(f::IeeeCompare::Lt, x(0)?, x(1)?)),
        FloatOp::IeeeLe => Value::Bool(f::ieee_compare(f::IeeeCompare::Le, x(0)?, x(1)?)),
        FloatOp::IeeeGt => Value::Bool(f::ieee_compare(f::IeeeCompare::Gt, x(0)?, x(1)?)),
        FloatOp::IeeeGe => Value::Bool(f::ieee_compare(f::IeeeCompare::Ge, x(0)?, x(1)?)),
        FloatOp::IsNan => Value::Bool(x(0)?.is_nan()),
        FloatOp::IsFinite => Value::Bool(x(0)?.is_finite()),
        FloatOp::IsInfinite => Value::Bool(x(0)?.is_infinite()),
        FloatOp::IsSignNegative => Value::Bool(f::to_bits(x(0)?) >> 63 == 1),
        FloatOp::IsZero => Value::Bool(x(0)? == 0.0),
        FloatOp::IsPositiveZero => Value::Bool(f::to_bits(x(0)?) == 0),
        FloatOp::IsNegativeZero => Value::Bool(f::to_bits(x(0)?) == 1 << 63),
        FloatOp::ToBits => Value::Int(BigInt::from(f::to_bits(x(0)?))),
        FloatOp::ToIntExact => match f::to_int_exact(x(0)?) {
            Some(n) => some(Value::Int(n)),
            None => none(),
        },
        FloatOp::FromInt => match args.first() {
            Some(Value::Int(n)) => Value::F64(f::from_int(n)),
            _ => return Err(fault("`to_f64` applied to a value that is not an integer")),
        },
        FloatOp::ToText => Value::text(&f::render(x(0)?)),
        FloatOp::ToF32 => Value::F32(una_core::float32::from_f64(x(0)?)),
        FloatOp::ToF64 => return Err(fault("`to_f64` applied to an `F64`")),
    })
}

/// `parse_f32(t)` (ADR 0120 rule 5): the failures of `parse_f64`, the value rounded once to
/// binary32.
fn parse_f32<'p>(t: &str) -> Flow<'p> {
    match una_core::float32::parse(t) {
        Ok(x) => Ok(Value::F32(x)),
        Err(_) => parse_f64(t),
    }
}

/// An operation of `F32` (ADR 0120 rules 5 to 9), each O(1), each `F32` result canonical:
/// the routines of `una_core::float32`, which native code calls too.
fn float32_apply<'p>(op: FloatOp, args: &[Value<'p>]) -> Flow<'p> {
    use una_core::float::IeeeCompare;
    use una_core::float32 as f;
    let x = |i: usize| match args.get(i) {
        Some(Value::F32(x)) => Ok(*x),
        _ => Err(fault(format!(
            "`{}` applied to a value that is not an `F32`",
            op.as_str_32()
        ))),
    };
    Ok(match op {
        FloatOp::Add => Value::F32(f::add(x(0)?, x(1)?)),
        FloatOp::Sub => Value::F32(f::sub(x(0)?, x(1)?)),
        FloatOp::Mul => Value::F32(f::mul(x(0)?, x(1)?)),
        FloatOp::Div => Value::F32(f::div(x(0)?, x(1)?)),
        FloatOp::Neg => Value::F32(f::neg(x(0)?)),
        FloatOp::Eq => Value::Bool(f::equal(x(0)?, x(1)?)),
        FloatOp::Ne => Value::Bool(!f::equal(x(0)?, x(1)?)),
        FloatOp::Lt => Value::Bool(f::total_cmp(x(0)?, x(1)?).is_lt()),
        FloatOp::Le => Value::Bool(f::total_cmp(x(0)?, x(1)?).is_le()),
        FloatOp::Gt => Value::Bool(f::total_cmp(x(0)?, x(1)?).is_gt()),
        FloatOp::Ge => Value::Bool(f::total_cmp(x(0)?, x(1)?).is_ge()),
        FloatOp::Abs => Value::F32(f::abs(x(0)?)),
        FloatOp::Sqrt => Value::F32(f::sqrt(x(0)?)),
        FloatOp::Fma => Value::F32(f::fma(x(0)?, x(1)?, x(2)?)),
        FloatOp::Rem => Value::F32(f::rem(x(0)?, x(1)?)),
        FloatOp::Floor => Value::F32(f::floor(x(0)?)),
        FloatOp::Ceil => Value::F32(f::ceil(x(0)?)),
        FloatOp::Trunc => Value::F32(f::trunc(x(0)?)),
        FloatOp::RoundHalfEven => Value::F32(f::round_half_even(x(0)?)),
        FloatOp::RoundHalfAway => Value::F32(f::round_half_away(x(0)?)),
        FloatOp::CopySign => Value::F32(f::copy_sign(x(0)?, x(1)?)),
        FloatOp::IeeeMin => Value::F32(f::ieee_min(x(0)?, x(1)?)),
        FloatOp::IeeeMax => Value::F32(f::ieee_max(x(0)?, x(1)?)),
        FloatOp::Clamp => Value::F32(f::clamp(x(0)?, x(1)?, x(2)?)),
        FloatOp::IeeeEq => Value::Bool(f::ieee_compare(IeeeCompare::Eq, x(0)?, x(1)?)),
        FloatOp::IeeeNe => Value::Bool(f::ieee_compare(IeeeCompare::Ne, x(0)?, x(1)?)),
        FloatOp::IeeeLt => Value::Bool(f::ieee_compare(IeeeCompare::Lt, x(0)?, x(1)?)),
        FloatOp::IeeeLe => Value::Bool(f::ieee_compare(IeeeCompare::Le, x(0)?, x(1)?)),
        FloatOp::IeeeGt => Value::Bool(f::ieee_compare(IeeeCompare::Gt, x(0)?, x(1)?)),
        FloatOp::IeeeGe => Value::Bool(f::ieee_compare(IeeeCompare::Ge, x(0)?, x(1)?)),
        FloatOp::IsNan => Value::Bool(x(0)?.is_nan()),
        FloatOp::IsFinite => Value::Bool(x(0)?.is_finite()),
        FloatOp::IsInfinite => Value::Bool(x(0)?.is_infinite()),
        FloatOp::IsSignNegative => Value::Bool(f::to_bits(x(0)?) >> 31 == 1),
        FloatOp::IsZero => Value::Bool(x(0)? == 0.0),
        FloatOp::IsPositiveZero => Value::Bool(f::to_bits(x(0)?) == 0),
        FloatOp::IsNegativeZero => Value::Bool(f::to_bits(x(0)?) == 1 << 31),
        FloatOp::ToBits => Value::Int(BigInt::from(f::to_bits(x(0)?))),
        FloatOp::ToIntExact => match f::to_int_exact(x(0)?) {
            Some(n) => some(Value::Int(n)),
            None => none(),
        },
        FloatOp::FromInt => match args.first() {
            Some(Value::Int(n)) => Value::F32(f::from_int(n)),
            _ => return Err(fault("`to_f32` applied to a value that is not an integer")),
        },
        FloatOp::ToText => Value::text(&f::render(x(0)?)),
        FloatOp::ToF64 => Value::F64(f::to_f64(x(0)?)),
        FloatOp::ToF32 => return Err(fault("`to_f32` applied to an `F32`")),
    })
}

/// `append` and `set` of the prelude (`spec/14 §1`, `spec/17 §4`). They update the list in
/// place when they hold its only reference, which a binding at its last use gives up (#201),
/// and copy it otherwise: no other name ever sees the change.
fn update<'p>(name: &str, args: Vec<Value<'p>>, span: Span) -> Flow<'p> {
    let bad = || fault(format!("`{name}` applied to values of the wrong types"));
    let mut args = args.into_iter();
    let Some(Value::List(mut items)) = args.next() else {
        return Err(bad());
    };
    if name == "append" {
        let item = args.next().ok_or_else(bad)?;
        Rc::make_mut(&mut items).push(item);
        return Ok(Value::List(items));
    }
    // `set` has an `edit` parameter: it returns the new list and its result, `unit`
    // (`spec/17 §4`, `§8`). Its `requires` keeps the index in range.
    let (Some(Value::Int(index)), Some(item)) = (args.next(), args.next()) else {
        return Err(bad());
    };
    let slot = usize::try_from(&index)
        .ok()
        .filter(|&i| i < items.len())
        // At the call, as native code aborts (#480).
        .ok_or_else(|| {
            Stop::Abort(Abort::Fault {
                message: "`set` at an index outside the list, which its precondition excludes"
                    .to_owned(),
                span: Some(span),
            })
        })?;
    if let Some(cell) = Rc::make_mut(&mut items).get_mut(slot) {
        *cell = item;
    }
    Ok(Value::Tuple(Rc::new(vec![Value::List(items), Value::Unit])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_outside_the_list_faults_at_the_call() {
        // #480: the fault points at the `set` call, as native code's abort does.
        let mut sources = una_syntax::SourceDb::new();
        let span = Span::at(sources.add("src/s.una", b"set"), 0);
        let outcome = update(
            "set",
            vec![
                Value::List(Rc::new(vec![Value::int(1)])),
                Value::int(1),
                Value::int(9),
            ],
            span,
        );
        match outcome {
            Err(Stop::Abort(Abort::Fault { message, span: at })) => {
                assert_eq!(at, Some(span));
                assert!(
                    message.contains("`set` at an index outside the list"),
                    "{message}"
                );
            }
            _ => panic!("an out-of-range `set` did not fault"),
        }
        let inside = update(
            "set",
            vec![
                Value::List(Rc::new(vec![Value::int(1)])),
                Value::int(0),
                Value::int(9),
            ],
            span,
        );
        assert!(inside.is_ok());
    }

    #[test]
    fn slice_and_replace_outside_their_preconditions_fault_at_the_call() {
        // #693: `slice` outside `0 <= start <= end <= t.len` and `replace` with an empty
        // pattern stop at the call, as native code's abort does; inside, they compute.
        let program = Program::default();
        let index = Index::new(&program);
        let mut world = crate::host::NoWorld;
        let keys = BTreeMap::new();
        let mut sources = una_syntax::SourceDb::new();
        let span = Span::at(sources.add("src/t.una", b"slice"), 0);
        let mut machine = Machine::new(
            &index,
            &mut world,
            &[],
            &keys,
            Limits::depth(8),
            Clock::real(),
        );
        let slice =
            |start: i64, end: i64| vec![Value::text("abc"), Value::int(start), Value::int(end)];
        for (name, args, expected) in [
            ("slice", slice(-1, 1), "`slice` outside the text"),
            ("slice", slice(0, 4), "`slice` outside the text"),
            ("slice", slice(2, 1), "`slice` outside the text"),
            (
                "replace",
                vec![Value::text("abc"), Value::text(""), Value::text("x")],
                "`replace` with an empty pattern",
            ),
        ] {
            match machine.prelude(name, args, span) {
                Err(Stop::Abort(Abort::Fault { message, span: at })) => {
                    assert_eq!(at, Some(span), "{name}");
                    assert!(message.contains(expected), "{message}");
                }
                _ => panic!("`{name}` outside its precondition did not fault"),
            }
        }
        // The boundaries `0 == start == end`, `start == end == t.len`, and a scalar of two bytes
        // at the end, counted in scalars.
        fn text_of(flow: Flow<'_>) -> String {
            match flow {
                Ok(Value::Text(t)) => t.as_str().to_owned(),
                _ => panic!("`slice` inside its precondition did not compute"),
            }
        }
        assert_eq!(text_of(machine.prelude("slice", slice(0, 3), span)), "abc");
        assert_eq!(text_of(machine.prelude("slice", slice(3, 3), span)), "");
        assert_eq!(text_of(machine.prelude("slice", slice(0, 0), span)), "");
        let accented = vec![Value::text("a\u{e9}"), Value::int(1), Value::int(2)];
        assert_eq!(text_of(machine.prelude("slice", accented, span)), "\u{e9}");
        let past = vec![Value::text("a\u{e9}"), Value::int(0), Value::int(3)];
        assert!(matches!(
            machine.prelude("slice", past, span),
            Err(Stop::Abort(Abort::Fault { .. }))
        ));
        let replaced = machine.prelude(
            "replace",
            vec![Value::text("abc"), Value::text("b"), Value::text("x")],
            span,
        );
        assert!(matches!(replaced, Ok(Value::Text(t)) if t.as_str() == "axc"));
    }

    #[test]
    fn the_prelude_kinds_core_leaves_to_the_engines_are_named_by_the_shared_table() {
        // #415: a value of a kind Core does not declare has its fields' names from
        // `una_core::prelude_variants`, the table the native backend describes it by, and no
        // declared type, so no field of it is redacted on its own.
        let program = Program::default();
        let index = Index::new(&program);
        for kind in una_core::PRELUDE_ENGINE_KINDS {
            for (position, (variant, fields)) in una_core::prelude_variants(kind)
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                let ctor = Ctor {
                    kind: una_core::Path {
                        module: una_core::PRELUDE_MODULE.to_owned(),
                        name: kind.to_owned(),
                    },
                    name: (*variant).to_owned(),
                    index: position,
                };
                let named: Vec<(&str, Option<&Type>)> =
                    fields.iter().map(|(f, _)| (*f, None)).collect();
                assert_eq!(index.fields(&ctor), named, "{kind}.{variant}");
            }
        }
    }

    #[test]
    fn calls_stop_at_the_maximum_depth_or_when_the_stack_runs_short() {
        let program = Program::default();
        let index = Index::new(&program);
        let mut world = crate::host::NoWorld;
        let keys = BTreeMap::new();
        let mut sources = una_syntax::SourceDb::new();
        let span = Span::at(sources.add("a.una", b"module a\n"), 0);
        let mut machine = Machine::new(
            &index,
            &mut world,
            &[],
            &keys,
            Limits::depth(3),
            Clock::real(),
        );
        for _ in 0..3 {
            assert!(machine.enter(span).is_ok());
        }
        assert!(matches!(
            machine.enter(span),
            Err(Stop::Abort(Abort::Depth { limit: 3, .. }))
        ));
        // A stack used beyond the budget aborts in the same way, below the maximum depth.
        let mut world = crate::host::NoWorld;
        let mut machine = Machine::new(
            &index,
            &mut world,
            &[],
            &keys,
            Limits::depth(3),
            Clock::real(),
        );
        machine.stack_base = stack_position(&span).wrapping_add(4096);
        machine.stack_budget = 1024;
        assert!(matches!(
            machine.enter(span),
            Err(Stop::Abort(Abort::Depth { limit: 3, .. }))
        ));
        machine.stack_budget = 1024 * 1024;
        assert!(machine.enter(span).is_ok());
    }

    #[test]
    fn a_call_after_the_run_aborted_ends_with_the_run_before_the_depth_check() {
        // A call whose check point let another task abort the whole run ends with the run
        // (`spec/27 §7`), even past the maximum depth: its task does not end alone with
        // U6003 after the run's end.
        let program = Program::default();
        let mut index = Index::new(&program);
        index.tasks = true;
        let mut world = crate::host::NoWorld;
        let keys = BTreeMap::new();
        let mut sources = una_syntax::SourceDb::new();
        let span = Span::at(sources.add("a.una", b"module a\n"), 0);
        let mut machine = Machine::new(
            &index,
            &mut world,
            &[],
            &keys,
            Limits::depth(3),
            Clock::real(),
        );
        for _ in 0..3 {
            assert!(machine.enter(span).is_ok());
        }
        // At the limit and before any abort, the call stops with U6003.
        assert!(matches!(
            machine.enter(span),
            Err(Stop::Abort(Abort::Depth { limit: 3, .. }))
        ));
        machine.run.abort_run(Abort::Memory { span, limit: 7 });
        assert!(matches!(
            machine.enter(span),
            Err(Stop::Abort(Abort::Memory { limit: 7, .. }))
        ));
        // Below the limit, too, the call does not start.
        machine.depth = 0;
        assert!(matches!(
            machine.enter(span),
            Err(Stop::Abort(Abort::Memory { limit: 7, .. }))
        ));
    }

    #[test]
    fn calls_stop_when_the_run_holds_more_than_its_memory_budget() {
        let program = Program::default();
        let index = Index::new(&program);
        let mut world = crate::host::NoWorld;
        let keys = BTreeMap::new();
        let mut sources = una_syntax::SourceDb::new();
        let span = Span::at(sources.add("a.una", b"module a\n"), 0);
        // The heap held 100 MiB when the run started, which does not count.
        let held = std::sync::atomic::AtomicUsize::new(100 << 20);
        let probe = || held.load(std::sync::atomic::Ordering::Relaxed);
        let limits = Limits {
            http: una_http::client::Settings::default(),
            max_depth: 10,
            max_memory: 2,
            held: &probe,
            stop_after: None,
            task_depth: 10,
            stop: None,
            kernels: &una_core::dense::variants::Kernels::BASELINE,
            serve: None,
            http_places: None,
        };
        let mut machine = Machine::new(&index, &mut world, &[], &keys, limits, Clock::real());
        held.store(
            (100 << 20) + (2 << 20),
            std::sync::atomic::Ordering::Relaxed,
        );
        assert!(machine.enter(span).is_ok());
        held.store(
            (100 << 20) + (2 << 20) + 1,
            std::sync::atomic::Ordering::Relaxed,
        );
        assert!(matches!(
            machine.enter(span),
            Err(Stop::Abort(Abort::Memory { limit: 2, .. }))
        ));
    }

    #[test]
    fn division_is_euclidean() {
        let check = |a: i64, b: i64, q: i64, r: i64| {
            assert_eq!(
                div_euclid(&BigInt::from(a), &BigInt::from(b)),
                (BigInt::from(q), BigInt::from(r)),
                "{a} div {b}"
            );
        };
        check(7, 2, 3, 1);
        check(-7, 2, -4, 1);
        check(7, -2, -3, 1);
        check(-7, -2, 4, 1);
        check(6, 3, 2, 0);
        check(0, -5, 0, 0);
    }
}
