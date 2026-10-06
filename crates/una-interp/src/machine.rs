//! The evaluation of Core IR: the rules of `spec/11 §4`, big-step, call-by-value, left to
//! right, with the capability environment, the world and the trace.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use num_bigint::{BigInt, Sign};
use una_core::{
    Atom, Base, Bound, CapOp, Ctor, Decl, Expr, FixedOp, Fun, HandlerOp, IntType, Label, Level,
    LoopSource, Name, PRELUDE_MODULE, Path, Prim, Program, Rhs, Source, Type, Use,
};
use una_syntax::Span;

use crate::host::{self, FileFault, Grant, World};
use crate::value::{Closure, Value, compare, equal};

/// The environment `ρ`: binders and their values, innermost last. Core binders are unique
/// within a function (`spec/09 §1`), so the last binding of a name is the one in scope.
pub(crate) type Env<'p> = Vec<(&'p str, Value<'p>)>;

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
}

/// The result of evaluating an expression.
pub(crate) type Flow<'p> = Result<Value<'p>, Stop<'p>>;

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

    /// The names of the fields of `ctor`, with their declared types when they are known.
    fn fields(&self, ctor: &Ctor) -> Vec<(&'p str, Option<&'p Type>)> {
        if ctor.kind.module == "prelude" {
            let name = match (ctor.kind.name.as_str(), ctor.name.as_str()) {
                ("Option", "Some") | ("Result", "Ok") => "value",
                ("Result", "Err") => "failure",
                _ => return Vec::new(),
            };
            return vec![(name, None)];
        }
        self.data
            .get(&(ctor.kind.module.as_str(), ctor.kind.name.as_str()))
            .and_then(|variants| variants.get(ctor.index))
            .map(|fields| fields.iter().map(|(f, t)| (*f, Some(*t))).collect())
            .unwrap_or_default()
    }

    /// The Una form of `value`. A field whose declared type is labeled above
    /// `public` is `<redacted:level>`.
    pub(crate) fn render(&self, value: &Value<'_>) -> String {
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
                Value::Bool(b) => out.push_str(&b.to_string()),
                Value::Text(t) => out.push_str(&una_core::quote(t)),
                Value::Unit => out.push_str("unit"),
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
                        let label = ty.map_or_else(Label::public, deep_label);
                        if label.is_public() || matches!(field, Value::Unit) {
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
                ty.is_some_and(|ty| !has_param(ty) && deep_label(ty).is_public())
            })
            .map(|(v, (name, _))| (name.to_owned(), self.render(v)))
            .collect()
    }
}

/// The join of the labels at every position of `ty` that holds data.
pub(crate) fn deep_label(ty: &Type) -> Label {
    match ty {
        Type::Base { base, label, .. } => match base {
            Base::Named { args, .. } => args
                .iter()
                .fold(label.clone(), |acc, arg| acc.join(&deep_label(arg))),
            _ => label.clone(),
        },
        Type::Fn { label, .. } => label.clone(),
    }
}

fn has_param(ty: &Type) -> bool {
    match ty {
        Type::Base { base, .. } => match base {
            Base::Param(_) | Base::Unknown => true,
            Base::Named { args, .. } => args.iter().any(has_param),
            _ => false,
        },
        Type::Fn { .. } => false,
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

fn prelude_ctor(kind: &str, name: &str, index: usize) -> Ctor {
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
        _ => &[],
    }
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
}

impl Limits<'_> {
    /// The depth `max_depth`, with a memory budget nothing measures: for a caller that does not
    /// count allocations.
    pub fn depth(max_depth: usize) -> Limits<'static> {
        Limits {
            max_depth,
            max_memory: usize::MAX,
            held: &unmeasured,
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

    /// `let name = f(xs, ...) in rest` where `f` is `append` or `set` of the prelude and `xs`
    /// is a binding at its last use: the binding gives up its list, so that the call, holding
    /// the only reference, updates it in place, as the native code does (#30, #201). A list
    /// something else holds is still copied, so no value ever changes under another name.
    fn updates_in_place(&mut self, rhs: &'p Rhs, rest: &'p Expr) -> Option<&'p str> {
        let Rhs::Call { function, args, .. } = rhs else {
            return None;
        };
        if function.module != PRELUDE_MODULE || !matches!(function.name.as_str(), "append" | "set")
        {
            return None;
        }
        let Some(Atom::Var(name)) = args.first() else {
            return None;
        };
        let twice = args
            .iter()
            .filter(|arg| matches!(arg, Atom::Var(other) if other == name))
            .count()
            > 1;
        (!twice && self.dead_after(name, rest)).then_some(name.as_str())
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
                                let Rhs::Call {
                                    function,
                                    args,
                                    span,
                                    ..
                                } = rhs
                                else {
                                    return Err(fault("an update that is not a call"));
                                };
                                let args = self.atoms(env, args)?;
                                if let Some((_, value)) =
                                    env.iter_mut().rev().find(|(n, _)| *n == moved)
                                {
                                    *value = Value::Unit;
                                }
                                self.call(function, args, &caps, *span)?
                            }
                            None => self.rhs(env, &caps, rhs, ty)?,
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
                    // The next iterations read the bindings the body reads, but for the item
                    // and the state, which each iteration binds anew.
                    let rebound = std::iter::once(var.as_ref())
                        .chain(state.iter())
                        .map(|(name, _)| name.as_str())
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
                    // A task's captures are never given up: the tasks after it read them.
                    self.later.push((expr, Vec::new()));
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
                    self.later.push((expr, Vec::new()));
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
                .iter()
                .rev()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| fault(format!("`{name}` is not bound")))?,
            Atom::Int(digits) => Value::Int(
                digits
                    .parse::<BigInt>()
                    .map_err(|_| fault(format!("`{digits}` is not an integer")))?,
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
                    current = self.iteration(env, caps, var, state, current, item, body)?;
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
                    current = self.iteration(env, caps, var, state, current, &item, body)?;
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
    /// returns the new state.
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
    ) -> Result<Vec<Value<'p>>, Stop<'p>> {
        let mark = env.len();
        env.push((var.0.as_str(), item.clone()));
        for ((name, _), value) in state.iter().zip(current) {
            env.push((name.as_str(), value));
        }
        let result = self.eval(env, caps, body);
        env.truncate(mark);
        match result? {
            Value::Tuple(items) if items.len() == state.len() => Ok(items.as_ref().clone()),
            _ => Err(fault("a loop body whose value is not its state")),
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
            Rhs::Prim { prim, args, .. } => prim_apply(*prim, &self.atoms(env, args)?),
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
                    result: deep_label(ty),
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
            Rhs::Lambda { params, body, .. } => Ok(Value::Closure(Rc::new(Closure {
                params: params.iter().map(|(name, _)| name).collect(),
                body,
                env: env.clone(),
                caps: caps.clone(),
            }))),
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
            Instance::Host { .. } => self.host(capability, op, &args),
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
            Err(Stop::Abort(_)) => EventOutcome::Aborted,
        };
        if let Some(event) = self.trace.get_mut(slot) {
            event.outcome = outcome;
        }
        result
    }

    /// A host operation, within the grants (`spec/14 §3`).
    fn host(&mut self, capability: &str, op: &str, args: &[Value<'p>]) -> Flow<'p> {
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
            ("Env", "get") => Ok(match self.world.env_var(&text(0)) {
                Some(value) => some(Value::text(&value)),
                None => none(),
            }),
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
                })
            }
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
            return update(name, args);
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
            // Ascending and stable (`spec/20 §6` rule 2).
            "sort" => {
                let mut items = list(0)?.as_ref().clone();
                items.sort_by(compare);
                Value::List(Rc::new(items))
            }
            "is_digits" => Value::Bool(text(0)?.chars().all(|c| c.is_ascii_digit())),
            "starts_with" => Value::Bool(text(0)?.starts_with(text(1)?.as_ref())),
            "trim" => Value::text(text(0)?.trim()),
            "to_int" => {
                let t = text(0)?;
                let digits = t.strip_prefix('-').unwrap_or(&t);
                match t.parse::<BigInt>() {
                    Ok(n) if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) => {
                        some(Value::Int(n))
                    }
                    _ => none(),
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
                Value::Bool(list(0)?.iter().any(|v| equal(v, &item)))
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
        Prim::Eq => Value::Bool(equal(arg(0)?, arg(1)?)),
        Prim::Ne => Value::Bool(!equal(arg(0)?, arg(1)?)),
        Prim::Lt => Value::Bool(compare(arg(0)?, arg(1)?).is_lt()),
        Prim::Le => Value::Bool(compare(arg(0)?, arg(1)?).is_le()),
        Prim::Gt => Value::Bool(compare(arg(0)?, arg(1)?).is_gt()),
        Prim::Ge => Value::Bool(compare(arg(0)?, arg(1)?).is_ge()),
        Prim::Len => match arg(0)? {
            Value::Text(t) => Value::Int(BigInt::from(t.chars().count())),
            Value::List(items) => Value::Int(BigInt::from(items.len())),
            _ => return Err(fault("`len` of a value that is neither a text nor a list")),
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

/// `append` and `set` of the prelude (`spec/14 §1`, `spec/17 §4`). They update the list in
/// place when they hold its only reference, which a binding at its last use gives up (#201),
/// and copy it otherwise: no other name ever sees the change.
fn update<'p>(name: &str, args: Vec<Value<'p>>) -> Flow<'p> {
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
        .ok_or_else(|| {
            fault("`set` at an index outside the list, which its precondition excludes")
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
