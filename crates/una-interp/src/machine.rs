//! The evaluation of Core IR: the rules of `spec/11 §4`, big-step, call-by-value, left to
//! right, with the capability environment, the world and the trace.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use num_bigint::{BigInt, Sign};
use una_core::json::Halt;
use una_core::{
    Atom, Bound, CapOp, Ctor, DecimalOp, Decl, ExitTag, Expr, FixedOp, FloatOp, Fun, HandlerOp,
    IntType, Label, Level, LoopSource, MapOp, Name, Path, Prim, Program, Rhs, Source, Type, Use,
};
use una_syntax::Span;

mod dense;

use crate::host::{self, FileFault, Grant, World};
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
}

impl Instance<'_> {
    fn id(&self) -> &str {
        match self {
            Instance::Host { id } | Instance::Handled { id, .. } => id,
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
    /// The path of the `together` task that performed it, as `2.1`, or empty outside any
    /// task (`spec/19 §7`).
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
    /// The run aborted before it ended.
    Aborted,
    /// A task was cancelled (`spec/19 §7` rule 3): the event that ends a started task after a
    /// failed one, which names no operation. The interpreter, which runs tasks one after
    /// another, never cancels one; a native run can.
    Cancelled,
    /// A loop of the host stopped at the start of an iteration (ADR 0116 rule 14): the driver's,
    /// when the run's stop came. Like `Cancelled`, it names no operation.
    Stop,
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
}

/// The result of evaluating an expression.
pub(crate) type Flow<'p> = Result<Value<'p>, Stop<'p>>;

mod seqs;

mod sets;

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
    /// `public` is `<redacted:level>`.
    pub(crate) fn render(&self, value: &Value<'_>) -> String {
        self.una_form(value, Form::Redacted)
    }

    /// The Una form of `value` with every field, which interpolation writes: its text carries
    /// the value's labels (`spec/11 §5`, ADR 0068).
    pub(crate) fn written(&self, value: &Value<'_>) -> String {
        self.una_form(value, Form::Written)
    }

    /// The Una form of `value` in `form`: an example's results and reasons, which edition `0.2`
    /// shows with every field (`spec/11 §5`, ADR 0078).
    pub(crate) fn shown(&self, value: &Value<'_>, form: Form) -> String {
        self.una_form(value, form)
    }

    /// The Una form of `value` (`spec/11 §5`), in `form`.
    fn una_form(&self, value: &Value<'_>, form: Form) -> String {
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
                    out.push_str(&text);
                    continue;
                }
                Task::Value(value) => value,
            };
            let mut parts: Vec<Task<'_, '_>> = Vec::new();
            match value {
                Value::Int(n) => out.push_str(&n.to_string()),
                Value::F64(x) => out.push_str(&una_core::float::render(*x)),
                Value::Decimal(c, s) => out.push_str(&una_core::decimal::render(c, *s)),
                Value::Bool(b) => out.push_str(&b.to_string()),
                Value::Text(t) => out.push_str(&una_core::quote(t)),
                Value::Unit => out.push_str("unit"),
                // Interpolation writes no bytes of a hash (ADR 0068, amendment of
                // 2026-10-06); traces, examples and diagnostics show them.
                Value::Hashed(_) if form == Form::Written => out.push_str("<hashed>"),
                Value::Hashed(bytes) => out.push_str(&format!("hashed:{}", host::hex(bytes))),
                Value::Closure(_) => out.push_str("<function>"),
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
                // The construction call that rebuilds a dense value, its elements in
                // row-major order (ADR 0120 rule 41), each a task like a list's, so a deep
                // value never recurses on the native stack.
                Value::Dense(dense) => {
                    let open = match (dense.kind, dense.shape.as_slice()) {
                        (crate::value::DenseKind::Matrix, [rows, columns]) => {
                            format!("matrix(rows: {rows}, columns: {columns}, elements: [")
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
                Value::Data(data) => {
                    let names = self.fields(&data.ctor);
                    if data.fields.is_empty() {
                        out.push_str(&data.ctor.name);
                        continue;
                    }
                    parts.push(Task::Text(format!("{}(", data.ctor.name)));
                    let named = data.fields.len() > 1;
                    for (i, field) in data.fields.iter().enumerate() {
                        let (name, ty) = names.get(i).copied().unwrap_or(("?", None));
                        if i > 0 {
                            parts.push(Task::Text(", ".to_owned()));
                        }
                        if named {
                            parts.push(Task::Text(format!("{name}: ")));
                        }
                        let label = ty.map_or_else(Label::public, Type::deep_label);
                        if form != Form::Redacted
                            || label.is_public()
                            || matches!(field, Value::Unit)
                        {
                            parts.push(Task::Value(field));
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
        out
    }

    pub(crate) fn redacted(&self, value: &Value<'_>, label: &Label) -> String {
        if label.is_public() || matches!(value, Value::Unit) {
            self.render(value)
        } else {
            format!("<redacted:{}>", level_name(label.level))
        }
    }

    /// The variant of a failure and its fields whose declared labels are `public`, in their Una
    /// form (`spec/04 §8`). A field of a type parameter is not shown, since its label is not
    /// known at run time.
    pub(crate) fn public_fields(&self, value: &Value<'_>) -> Vec<(String, String)> {
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
            .map(|(v, (name, _))| (name.to_owned(), self.render(v)))
            .collect()
    }
}

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
/// the step budget in milliseconds, and the count of iterations after which the run stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drive {
    /// The tick period, from 1 millisecond.
    pub ticks: u64,
    /// The step budget, when one is set.
    pub budget: Option<u64>,
    /// `--stop-after-iterations`: the run stops when this many steps have started.
    pub stop_after: Option<u64>,
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

/// The Una Abstract Machine: one run of `main` or of one example.
pub(crate) struct Machine<'p, 'w> {
    index: &'w Index<'p>,
    world: &'w mut dyn World,
    grants: &'w [Grant],
    keys: &'w BTreeMap<String, Vec<u8>>,
    /// The limits and deadline of each HTTP request.
    http: una_http::client::Settings,
    /// The trace so far.
    pub(crate) trace: Vec<Event>,
    depth: usize,
    max_depth: usize,
    /// The memory budget in MiB, the probe of the heap, and what the heap held when the run
    /// started, which does not count.
    max_memory: usize,
    held: &'w dyn Fn() -> usize,
    baseline: usize,
    /// Where the evaluator's stack starts, and how much of it evaluation may use.
    stack_base: usize,
    stack_budget: usize,
    installs: BTreeMap<String, usize>,
    /// The name spans of the functions in progress, innermost last, to locate faults.
    frames: Vec<Span>,
    /// The path of the `together` task in progress, one 1-based index per enclosing block
    /// (`spec/19 §6`).
    task: Vec<usize>,
    /// What the function in progress evaluates after the expression in progress, innermost
    /// last, with the names that each rebinds before reading them: a binding none of them
    /// reads is dead once the expression no longer reads it (#201).
    later: Vec<(&'p Expr, Vec<&'p str>)>,
    /// `later` of the functions that called the one in progress.
    callers: Vec<Vec<(&'p Expr, Vec<&'p str>)>>,
    /// The names each expression reads, computed once (`una_core::vars`).
    reads: HashMap<*const Expr, Rc<BTreeSet<String>>>,
    /// The names each loop body binds, computed once (`una_core::binders`).
    binders: HashMap<*const Expr, Rc<BTreeSet<&'p str>>>,
}

impl<'p, 'w> Machine<'p, 'w> {
    /// A machine over `index`, with `world`, `grants` and the keys of `keys:` grants for host
    /// capabilities, and the limits of the run.
    pub(crate) fn new(
        index: &'w Index<'p>,
        world: &'w mut dyn World,
        grants: &'w [Grant],
        keys: &'w BTreeMap<String, Vec<u8>>,
        limits: Limits<'w>,
    ) -> Machine<'p, 'w> {
        let max_depth = limits.max_depth;
        Machine {
            index,
            world,
            grants,
            keys,
            trace: Vec::new(),
            depth: 0,
            max_depth,
            max_memory: limits.max_memory,
            http: limits.http,
            held: limits.held,
            baseline: (limits.held)(),
            stack_base: stack_position(&max_depth),
            stack_budget: crate::stack_bytes(max_depth)
                .map_or(usize::MAX, |bytes| bytes.saturating_sub(STACK_RESERVE)),
            installs: BTreeMap::new(),
            frames: Vec::new(),
            task: Vec::new(),
            later: Vec::new(),
            callers: Vec::new(),
            reads: HashMap::new(),
            binders: HashMap::new(),
        }
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
        let result = self.eval(&mut Env::new(), &Rc::new(caps), &main.body);
        let result = self.locate(result);
        self.frames.pop();
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
    /// `drive.ticks` milliseconds on the monotonic clock, delivering the latest due tick with the
    /// count of the due ticks skipped before it, then `on_stop` when one is declared. The loop
    /// stops at the start of an iteration once `drive.stop_after` iterations have started (the
    /// stop of `--stop-after-iterations`, rule 19); until host stops land, that is its only
    /// stop. A failure of `init` ends the run as one of `main` does; `step` and `on_stop`
    /// cannot fail (U3029).
    pub(crate) fn drive(
        &mut self,
        init: &'p Fun,
        step: &'p Fun,
        on_stop: Option<&'p Fun>,
        drive: &Drive,
    ) -> Flow<'p> {
        let mut state = self.call_hosted(init, Vec::new())?;
        let period = std::time::Duration::from_millis(drive.ticks);
        let start = std::time::Instant::now();
        let mut next: u64 = 0;
        let mut iterations: u64 = 0;
        let ended = loop {
            if drive.stop_after.is_some_and(|limit| iterations >= limit) {
                let slot = self.trace.len();
                self.trace.push(Event {
                    seq: slot + 1,
                    task: String::new(),
                    instance: String::new(),
                    capability: String::new(),
                    op: String::new(),
                    args: Vec::new(),
                    outcome: EventOutcome::Stop,
                });
                break Ok(());
            }
            iterations += 1;
            // Tick `k` is due `k * ticks` after the first: wait for the next one, or deliver the
            // latest one due, with the count of those skipped.
            let due = |k: u64| start + period.saturating_mul(u32::try_from(k).unwrap_or(u32::MAX));
            let now = std::time::Instant::now();
            if now < due(next) {
                std::thread::sleep(due(next) - now);
            }
            let elapsed = start.elapsed().as_millis();
            let latest =
                u64::try_from(elapsed / u128::from(drive.ticks.max(1))).unwrap_or(u64::MAX);
            let latest = latest.max(next);
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
            // `step(edit state, input)` returns `(state', unit)` in Core (`spec/17 §3`).
            match self.call_hosted(step, vec![state.clone(), tick]) {
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

    /// Whether the heap the run holds is within its budget (ADR 0061): checked at each call
    /// and at each iteration of a `for` loop, at `span`, else the run aborts with U6003.
    fn within_budget(&self, span: Span) -> Result<(), Stop<'p>> {
        let budget = self.max_memory.saturating_mul(1024 * 1024);
        if (self.held)().saturating_sub(self.baseline) > budget {
            return Err(Stop::Abort(Abort::Memory {
                span,
                limit: self.max_memory,
            }));
        }
        Ok(())
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
            self.reads
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
            self.binders
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
        let later = self.later.clone();
        later
            .into_iter()
            .all(|(expr, rebound)| rebound.contains(&name) || !self.reads(expr).contains(name))
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
                    span,
                } => {
                    let mut current = self.atoms(env, init)?;
                    // The next iterations read the bindings the body reads, but for the item,
                    // the state and the binders of the body, which each iteration binds anew
                    // before reading them: Core binders are unique (#261).
                    let rebound = std::iter::once(var.as_ref())
                        .chain(state.iter())
                        .map(|(name, _)| name.as_str())
                        .chain(self.binders(body).iter().copied())
                        .collect();
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
                    self.later.pop();
                    current = result?;
                    return Ok(Value::Tuple(Rc::new(current)));
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
                    for (index, task) in tasks.iter().enumerate() {
                        match self.task(env, &caps, index, task) {
                            Ok(result) => results.push(result),
                            Err(stop) => {
                                self.later.pop();
                                return Err(stop);
                            }
                        }
                    }
                    self.later.pop();
                    return Ok(Value::Tuple(Rc::new(results)));
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
                    for (index, item) in items.into_iter().enumerate() {
                        let mark = env.len();
                        env.push((var.0.as_str(), item));
                        let result = self.task(env, &caps, index, task);
                        env.truncate(mark);
                        match result {
                            Ok(result) => results.push(result),
                            Err(stop) => {
                                self.later.pop();
                                return Err(stop);
                            }
                        }
                    }
                    self.later.pop();
                    return Ok(Value::List(Rc::new(results)));
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
                    for ((name, _), item) in names.iter().zip(items.iter()) {
                        env.push((name.as_str(), item.clone()));
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
                let info = self
                    .index
                    .handlers
                    .get(&(handler.module.as_str(), handler.name.as_str()))
                    .copied()
                    .ok_or_else(|| fault(format!("no handler `{}`", handler.qualified())))?;
                let id = if *given {
                    format!("given:{cap}")
                } else {
                    let count = self.installs.entry(handler.name.clone()).or_insert(0);
                    *count += 1;
                    format!("with:{}#{count}", handler.name)
                };
                Ok(Rc::new(Instance::Handled {
                    handler: info,
                    values,
                    caps: restrict(caps, info.uses),
                    id,
                }))
            }
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
            LoopSource::List(list) => {
                let Value::List(items) = self.atom(env, list)? else {
                    return Err(fault("a loop over a value that is not a list"));
                };
                for item in items.iter() {
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
            Ok(Value::Tuple(items)) if items.len() == state.len() => {
                Ok((items.as_ref().clone(), false))
            }
            Err(Stop::Exit(tag, values)) if values.len() == state.len() => {
                Ok((values, tag == ExitTag::Stop))
            }
            Err(Stop::Exit(..)) | Ok(_) => Err(fault("a loop body whose value is not its state")),
            Err(stop) => Err(stop),
        }
    }

    /// Task `index` of a `together` block, `task`, with its path (`spec/19 §6` rule 1).
    fn task(
        &mut self,
        env: &mut Env<'p>,
        caps: &Caps<'p>,
        index: usize,
        task: &'p Expr,
    ) -> Flow<'p> {
        self.task.push(index + 1);
        let mark = env.len();
        let result = self.eval(env, caps, task);
        env.truncate(mark);
        self.task.pop();
        result
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
                if let (Prim::Eq | Prim::Ne, [a, b]) = (*prim, args.as_slice()) {
                    let same = self.equal_at(a, b, *span)?;
                    return Ok(Value::Bool(same == (*prim == Prim::Eq)));
                }
                // Interpolation writes a structured value in its Una form (ADR 0068).
                if *prim == Prim::ToText
                    && let Some(value) = args.first()
                    && !matches!(value, Value::Int(_) | Value::Bool(_) | Value::Text(_))
                {
                    return Ok(Value::text(&self.index.written(value)));
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
                if let Some(made) = self.collections(&function.name) {
                    return made;
                }
            }
            "std.testing" if function.name == "fake_hash" => {
                return match args.first() {
                    Some(Value::Int(seed)) => {
                        Ok(Value::Hashed(Rc::from(host::fake_hash(&seed.to_string()))))
                    }
                    _ => Err(fault("`fake_hash` without an `Int` seed")),
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

    /// A function of `std.collections` whose body is native (`spec/28`, ADR 0119), or `None`
    /// for one the module declares in Una.
    fn collections(&mut self, name: &str) -> Option<Flow<'p>> {
        Some(Ok(match name {
            // O(1): an empty ring buffer allocates nothing.
            "empty_stack" | "empty_queue" => Value::Seq(Rc::default()),
            _ => return None,
        }))
    }

    /// A function of `std.collections` whose body is native (`spec/28`, ADR 0119), or `None`
    /// for one the module declares in Una.
    fn collections(&mut self, name: &str) -> Option<Flow<'p>> {
        Some(Ok(match name {
            // O(1).
            "empty_set" => Value::Set(Rc::default()),
            _ => return None,
        }))
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
        let instance = caps
            .get(cap)
            .cloned()
            .ok_or_else(|| fault(format!("no capability `@{cap}`")))?;
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
        let shown: Vec<(String, String)> = args
            .iter()
            .enumerate()
            .map(|(i, value)| {
                let name = names.get(i).cloned().unwrap_or_else(|| format!("arg{i}"));
                // An argument labeled above `public` is redacted.
                let label = labels.get(i).cloned().unwrap_or_else(Label::public);
                (name, self.index.redacted(value, &label))
            })
            .collect();
        let slot = self.trace.len();
        let task: Vec<String> = self.task.iter().map(ToString::to_string).collect();
        self.trace.push(Event {
            seq: slot + 1,
            task: task.join("."),
            instance: instance.id().to_owned(),
            capability: capability.to_owned(),
            op: op.to_owned(),
            args: shown,
            outcome: EventOutcome::Aborted,
        });
        let result = match instance.as_ref() {
            Instance::Host { .. } => self.host(capability, op, &args, span),
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
                let result = self.eval(&mut env, own, &implementation.body);
                self.leave(result)
            }
        };
        let outcome = match &result {
            Ok(value) => EventOutcome::Ok(self.index.redacted(value, &request.result)),
            Err(Stop::Fail(failure)) => EventOutcome::Fail {
                variant: variant(failure),
                value: self.index.render(failure),
            },
            Err(Stop::Abort(_) | Stop::Exit(..)) => EventOutcome::Aborted,
        };
        if let Some(event) = self.trace.get_mut(slot) {
            event.outcome = outcome;
        }
        result
    }

    /// An operation of `Http` (`spec/24`), by the client that native code shares.
    fn http(&mut self, op: &str, args: &[Value<'p>], span: Span) -> Flow<'p> {
        use una_http::client::{Error, Request, send};
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
        let grants: Vec<una_core::http::Grant> = self
            .grants
            .iter()
            .filter_map(|grant| match grant {
                Grant::Http(http) => Some(http.clone()),
                _ => None,
            })
            .collect();
        let request = Request {
            method,
            url: &url,
            headers: &headers,
            body: body.as_bytes(),
            bearer: bearer.as_deref(),
        };
        let response = send(&request, &grants, &self.http).map_err(fail)?;
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
                self.world.print(&text(0));
                Ok(Value::Unit)
            }
            ("Clock", "now") => Ok(Value::int(self.world.now())),
            // A value that is not valid UTF-8 aborts the run: its content, not the name,
            // would decide the `Option`'s `public` tag (S-16, ADR 0086).
            ("Env", "get") => match self.world.env_var(&text(0)) {
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
                let mut target = None;
                for grant in self.grants {
                    if let Grant::Files {
                        prefix,
                        read,
                        write,
                    } = grant
                        && (if writing { *write } else { *read })
                    {
                        target = host::confine(self.world, prefix, &path);
                        if target.is_some() {
                            break;
                        }
                    }
                }
                let Some(target) = target else {
                    return Err(file_error("Denied", 1));
                };
                let done = if writing {
                    self.world
                        .write_file(&target, &text(1))
                        .map(|()| Value::Unit)
                } else {
                    self.world.read_file(&target).map(|t| Value::text(&t))
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
                let tree = match crate::json::tree(&arg(0)?, &mut within) {
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
    /// The label of the binder of its result, which redacts its outcome.
    result: Label,
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
            // An array's length (`spec/29 §17`).
            Value::Dense(dense) => {
                Value::Int(BigInt::from(dense.shape.first().copied().unwrap_or(0)))
            }
            _ => {
                return Err(fault(
                    "`len` of a value that is not a text, list, map, sequence, set or array",
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
        Prim::Float(op) => return float_apply(op, args),
        Prim::Decimal(op) => return decimal_apply(op, args),
        // The shape of a matrix (`spec/29 §17`).
        Prim::Shape(fun) => match arg(0)? {
            Value::Dense(dense) => {
                let axis = match fun {
                    una_core::ShapeFun::Rows => 0,
                    una_core::ShapeFun::Columns => 1,
                };
                // Exact, as `len` is: a dimension is the one the program gave.
                Value::Int(BigInt::from(dense.shape.get(axis).copied().unwrap_or(0)))
            }
            _ => return Err(fault("a shape of a value that is not a matrix")),
        },
        Prim::ToText => match arg(0)? {
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
                BigInt::from_biguint(num_bigint::Sign::Plus, x.magnitude().clone()),
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
        let mut machine = Machine::new(&index, &mut world, &[], &keys, Limits::depth(8));
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
        let mut machine = Machine::new(&index, &mut world, &[], &keys, Limits::depth(3));
        for _ in 0..3 {
            assert!(machine.enter(span).is_ok());
        }
        assert!(matches!(
            machine.enter(span),
            Err(Stop::Abort(Abort::Depth { limit: 3, .. }))
        ));
        // A stack used beyond the budget aborts in the same way, below the maximum depth.
        let mut world = crate::host::NoWorld;
        let mut machine = Machine::new(&index, &mut world, &[], &keys, Limits::depth(3));
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
        };
        let mut machine = Machine::new(&index, &mut world, &[], &keys, limits);
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
