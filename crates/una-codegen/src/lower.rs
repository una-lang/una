//! The body of one function of Core IR, as Cranelift instructions over words (`spec/16 §4`).
//!
//! **Reference counting** (NB-2). A word that a binder holds is either owned, and released at
//! the end of the binder's scope, or borrowed from a value that outlives the binder:
//! - a `let` owns what its right-hand side makes, and every right-hand side returns an owned
//!   word, retaining a borrowed one;
//! - parameters, captured values and the fields that a `case` arm binds are borrowed from the
//!   caller, the closure and the scrutinee, except the parameters a function owns (below);
//! - literals are borrowed from the runtime, which keeps them for the whole run;
//! - calls borrow their arguments, but for owned parameters, and return an owned word.
//!
//! Each path through an expression ends with its result: it retains the result, then releases
//! every word its scope owns.
//!
//! **Moves** (#30, `spec/16 §4` rule 7). At its last use in a path, a word the scope owns moves
//! instead of being borrowed: to `let b = a`, out as the result, into `append` and `set`, which
//! update a list in place when nothing else holds it, into a record of its kind rebuilt there,
//! into a loop as its initial state, or into a nested expression that uses it last. A loop's
//! iteration owns its state. A binding something borrows from (a `when` scrutinee, a `let (...)`
//! tuple, a list a loop visits) never moves.
//!
//! **Owned parameters** (ADR 0072, #262). A function of the package owns the parameters it
//! consumes, by borrow inference (`owned`): a call gives each of them a reference, by moving
//! an argument at its last use, or by retaining it otherwise. An owned parameter belongs to
//! the body's scope, which releases or moves it like a `let` binding.
//!
//! **Failures** are returned values (`spec/16 §4` rule 3). A body returns its value and a flag,
//! 1 when the value is a failure. A failure releases what the code owns since the innermost
//! [`Catch`] was installed, and goes to it: a handling clause, `attempt`, or the body's return.

use std::collections::{BTreeMap, BTreeSet};

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{AbiParam, Block, InstBuilder, Value, types};
use cranelift_codegen::isa::TargetFrontendConfig;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use una_core::{
    Atom, Base, Bound, Constraint, Ctor, Decl, Descriptor, ExitTag, Expr, HOST_MODULE, Label,
    LoopSource, MapOp, Name, PRELUDE_MODULE, Path, Prim, Rhs, Source, Type, vars,
};
use una_syntax::Span;

use crate::{CodegenError, Emitter, describe, range};

/// The integers a tagged word holds: `[-2^62, 2^62)`.
const SMALL: std::ops::RangeInclusive<i64> = -(1 << 62)..=(1 << 62) - 1;

/// What a word holds, as far as the code needs to know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A tagged integer or a handle to a big one.
    Int,
    /// 0 or 1.
    Bool,
    /// A handle to a text.
    Text,
    /// 0.
    Unit,
    /// A variant or a record: an immediate without fields, else a handle.
    Data,
    /// A handle to a list.
    List,
    /// A handle to a map (`spec/21`).
    Map,
    /// A handle to a closure, or to an instance of a capability.
    Fn,
    /// A handle to a keyed hash.
    Hashed,
    /// A handle to a reply right of `std.host` (ADR 0123 rule 4): the runtime's word of the
    /// run's table, which `==` compares and interpolation writes `<reply-right>`.
    Right,
    /// A handle to a boxed `F64` (ADR 0114 rule 9.6).
    F64,
    /// An `F32`: the tagged word of the key of totalOrder of its canonical bits, which fits 63
    /// bits, so it never allocates (ADR 0120 rule 10). Never a handle.
    F32,
    /// A handle to a tuple: the state of a loop, or the results of a call with `edit`
    /// parameters (`spec/17 §4`).
    Tuple,
    /// A handle to a dense value, an `Array`, `Matrix`, `Tensor` or `Lu` (`spec/29 §17`, ADR
    /// 0120 rule 28): the runtime's header over its store.
    Dense,
    /// A handle to a collection of `std.collections`: a `Stack`, a `Queue`, a `Set`, a
    /// `HashMap`, a `HashSet`, a `PriorityQueue` or a `BitSet` (ADR 0119 rule 19).
    Coll,
    /// A value of a type parameter: any of the above.
    Any,
}

impl Kind {
    /// Whether the word may be a handle, which reference counting tracks.
    fn counted(self) -> bool {
        !matches!(self, Kind::Bool | Kind::Unit | Kind::F32)
    }

    /// Whether the word is always a handle.
    fn handle(self) -> bool {
        matches!(
            self,
            Kind::Text
                | Kind::List
                | Kind::Map
                | Kind::Fn
                | Kind::Hashed
                | Kind::Right
                | Kind::Tuple
                | Kind::F64
                | Kind::Dense
                | Kind::Coll
        )
    }
}

fn unsupported(what: impl Into<String>) -> CodegenError {
    CodegenError::Unsupported(what.into())
}

fn internal(what: impl Into<String>) -> CodegenError {
    CodegenError::Internal(what.into())
}

/// The runtime's code of the collection operation `prim`, its position in its kind's `ALL`, and
/// whether it gives a new collection built from its receiver (ADR 0119 rule 1).
fn collection_code(prim: Prim) -> Result<(i64, bool), CodegenError> {
    fn position<T: PartialEq>(all: &[T], op: &T) -> Option<i64> {
        all.iter()
            .position(|x| x == op)
            .and_then(|i| i64::try_from(i).ok())
    }
    let found = match prim {
        Prim::Seq(op) => position(&una_core::SeqOp::ALL, &op).map(|code| (code, op.rebuilds())),
        Prim::Set(op) => position(&una_core::SetOp::ALL, &op).map(|code| (code, op.rebuilds())),
        Prim::Priority(op) => {
            position(&una_core::PriorityOp::ALL, &op).map(|code| (code, op.rebuilds()))
        }
        Prim::BitSet(op) => {
            position(&una_core::BitSetOp::ALL, &op).map(|code| (code, op.rebuilds()))
        }
        Prim::HashMap(op) => {
            position(&una_core::HashMapOp::ALL, &op).map(|code| (code, op.rebuilds()))
        }
        Prim::HashSet(op) => {
            position(&una_core::HashSetOp::ALL, &op).map(|code| (code, op.rebuilds()))
        }
        _ => None,
    };
    found.ok_or_else(|| internal(format!("`{}` is no collection operation", prim.as_str())))
}

/// The kind of a value of type `ty`, or what the backend does not compile yet.
fn kind(ty: &Type) -> Result<Kind, CodegenError> {
    match ty {
        Type::Base { base, .. } => match base {
            Base::Int => Ok(Kind::Int),
            Base::Bool => Ok(Kind::Bool),
            Base::Text => Ok(Kind::Text),
            Base::Unit => Ok(Kind::Unit),
            Base::Hashed => Ok(Kind::Hashed),
            Base::ReplyRight => Ok(Kind::Right),
            // A fixed-width integer is an integer word: its range is proven, so arithmetic on
            // it never leaves the type (`spec/18 §3`).
            Base::Fixed(_) => Ok(Kind::Int),
            // Boxed until the raw cells of ADR 0107 land (ADR 0114 rule 9.6).
            Base::F64 => Ok(Kind::F64),
            // A decimal is its coefficient, an integer word; its scale is static (ADR 0120
            // rule 20).
            Base::Decimal(_) => Ok(Kind::Int),
            Base::F32 => Ok(Kind::F32),
            Base::Named { module, name, .. } if module == PRELUDE_MODULE && name == "List" => {
                Ok(Kind::List)
            }
            Base::Named { module, name, .. } if module == PRELUDE_MODULE && name == "Map" => {
                Ok(Kind::Map)
            }
            // A counted handle to the runtime's header (`spec/29 §17`, ADR 0120 rule 28).
            Base::Named { module, name, .. }
                if module == PRELUDE_MODULE
                    && matches!(name.as_str(), "Array" | "Matrix" | "Tensor" | "Lu") =>
            {
                Ok(Kind::Dense)
            }
            // The collections of `std.collections`: one object each, with a layout of its own in
            // the runtime (`spec/28`, ADR 0119 rule 19).
            Base::Named { module, name, .. }
                if module == una_core::COLLECTIONS_MODULE
                    && una_core::Coll::from_name(name).is_some() =>
            {
                Ok(Kind::Coll)
            }
            Base::Tuple(_) => Ok(Kind::Tuple),
            Base::Named { .. } => Ok(Kind::Data),
            Base::Param(_) | Base::Unknown => Ok(Kind::Any),
        },
        Type::Fn { .. } => Ok(Kind::Fn),
    }
}

/// Whether the prelude function `name` is a constructor or a method of dense values
/// (`spec/29 §17`), as the interpreter's `is_dense`.
pub(crate) fn is_dense(name: &str) -> bool {
    matches!(
        name,
        "array"
            | "array_filled"
            | "matrix"
            | "matrix_filled"
            | "matrix_diagonal"
            | "tensor"
            | "tensor_filled"
    ) || name.starts_with("dense_")
}

/// The method of the dense prelude function `name` after its type's prefix, without a
/// tensor's rank: `dense_tensor_get_3` is `get`, as the interpreter's `method_of`.
pub(crate) fn dense_method(name: &str) -> &str {
    let method = [
        "dense_array_",
        "dense_matrix_",
        "dense_tensor_",
        "dense_lu_",
    ]
    .iter()
    .find_map(|prefix| name.strip_prefix(prefix))
    .unwrap_or(name);
    match method.rsplit_once('_') {
        Some((base, rank))
            if name.starts_with("dense_tensor_")
                && rank
                    .parse::<usize>()
                    .is_ok_and(|rank| (1..=8).contains(&rank)) =>
        {
            base
        }
        _ => method,
    }
}

/// Whether the dense prelude function `name` takes its receiver by `edit` (ADR 0120 rule 31):
/// at the receiver's last use it moves in, and is updated in place when held once.
pub(crate) fn dense_edits(name: &str) -> bool {
    name.starts_with("dense_")
        && matches!(
            dense_method(name),
            "set" | "update" | "add_product" | "add_scaled" | "scale"
        )
}

/// The binding of the type descriptor of the type parameter `param` (`spec/20 §7`): `$T`, which
/// no source name can be.
pub(crate) fn descriptor_name(param: &str) -> String {
    format!("${param}")
}

/// The type of a type descriptor's binding: an immediate, which reference counting skips.
pub(crate) fn descriptor_type() -> Type {
    base(Base::Bool)
}

/// The descriptor that a type parameter under `constraint` takes when `ty` instantiates it, or
/// `None` when `ty` is a type parameter, whose own descriptor is passed on (`spec/20 §7`). A
/// sum's and a structural descriptor carry the index of the type's description, by `index`,
/// which interpolation writes the value by (ADR 0068).
pub(crate) fn constant_descriptor(
    ty: &Type,
    constraint: Constraint,
    index: &mut dyn FnMut(&Type) -> usize,
) -> Option<Descriptor> {
    let Type::Base { base, .. } = ty else {
        return Some(Descriptor::Structural(index(ty)));
    };
    Some(match base {
        Base::Param(_) => return None,
        // A type that holds a type parameter takes a composite descriptor, built at run time
        // from the caller's descriptors (ADR 0068 D6).
        _ if constraint < Constraint::Ord && !type_params(ty).is_empty() => return None,
        Base::Int => Descriptor::Int,
        Base::Fixed(int) => Descriptor::Fixed(*int),
        Base::Text => Descriptor::Text,
        Base::F64 => Descriptor::F64,
        Base::Decimal(_) => Descriptor::Decimal(index(ty)),
        Base::F32 => Descriptor::F32,
        // A kind satisfies `Ord` only as a sum whose variants have no payload (`spec/20 §1`).
        Base::Named { module, .. } if module != PRELUDE_MODULE && constraint >= Constraint::Ord => {
            Descriptor::Sum(index(ty))
        }
        _ => Descriptor::Structural(index(ty)),
    })
}

/// The type parameters that `ty` holds, by first occurrence.
fn type_params(ty: &Type) -> Vec<String> {
    fn walk(ty: &Type, out: &mut Vec<String>) {
        match ty {
            Type::Base { base, .. } => match base {
                Base::Param(name) => {
                    if !out.contains(name) {
                        out.push(name.clone());
                    }
                }
                Base::Named { args, .. } => args.iter().for_each(|arg| walk(arg, out)),
                Base::Tuple(items) => items.iter().for_each(|item| walk(item, out)),
                _ => {}
            },
            Type::Fn { params, result, .. } => {
                params.iter().for_each(|param| walk(param, out));
                walk(result, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(ty, &mut out);
    out
}

/// A type of no known base: a value the code only passes on.
fn unknown() -> Type {
    base(Base::Unknown)
}

fn base(base: Base) -> Type {
    Type::Base {
        base,
        refinements: Vec::new(),
        label: Label::public(),
    }
}

/// `ty` with each type parameter of `params` replaced by its argument in `args`.
fn substitute(ty: &Type, params: &[String], args: &[Type]) -> Type {
    match ty {
        Type::Base {
            base: Base::Param(name),
            ..
        } => params
            .iter()
            .position(|p| p == name)
            .and_then(|i| args.get(i))
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        Type::Base {
            base:
                Base::Named {
                    module,
                    name,
                    args: inner,
                },
            refinements,
            label,
        } => Type::Base {
            base: Base::Named {
                module: module.clone(),
                name: name.clone(),
                args: inner.iter().map(|t| substitute(t, params, args)).collect(),
            },
            refinements: refinements.clone(),
            label: label.clone(),
        },
        Type::Base { .. } => ty.clone(),
        Type::Fn {
            params: inner,
            result,
            uses,
            fails,
            label,
        } => Type::Fn {
            params: inner.iter().map(|t| substitute(t, params, args)).collect(),
            result: Box::new(substitute(result, params, args)),
            uses: uses.clone(),
            fails: fails.clone(),
            label: label.clone(),
        },
    }
}

/// The binders in scope, with their words and types, and the capabilities in scope, keyed
/// `@name`, with their instances.
type Env = BTreeMap<String, (Value, Type)>;

/// The key of the capability `name` in an [`Env`].
fn cap_key(name: &str) -> String {
    format!("@{name}")
}

/// The words a scope owns, released at the end of each of its paths.
type Scope = Vec<(Value, Kind)>;

/// Where a failure goes, as a returned value that nothing unwinds (`spec/16 §4` rule 3): the
/// block of the innermost handling clause, `attempt` or function return, whose parameter is the
/// failure, with the words to release on the way.
#[derive(Clone)]
struct Catch {
    /// The block.
    block: Block,
    /// The words that enclosing expressions own since the catch was installed.
    outer: Scope,
    /// Where the words of the current scope since the catch was installed start.
    base: usize,
    /// Where `skip` and `stop` go, inside a loop's body (`spec/26 §3`).
    exit: Option<Exit>,
}

/// The blocks that `skip` and `stop` of the innermost loop jump to, whose parameters are the
/// loop's state, with the words to release on the way: those of the body's scopes, never the
/// loop's own, which it keeps for its next iteration.
#[derive(Clone)]
struct Exit {
    /// The block of the next iteration.
    next: Block,
    /// The block that ends the loop.
    stop: Block,
    /// The kinds of the state.
    kinds: Vec<Kind>,
    /// The words that enclosing expressions of the body own.
    outer: Scope,
    /// Where the words of the current scope that the body owns start.
    base: usize,
}

impl Catch {
    /// The catch of an expression nested in the one whose scope is `scope`: the nested
    /// expression's scope starts empty.
    fn nested(&self, scope: &Scope) -> Catch {
        let mut outer = self.outer.clone();
        outer.extend(scope.iter().skip(self.base).copied());
        Catch {
            block: self.block,
            outer,
            base: 0,
            exit: self.exit.as_ref().map(|exit| {
                let mut outer = exit.outer.clone();
                outer.extend(scope.iter().skip(exit.base).copied());
                Exit {
                    outer,
                    base: 0,
                    ..exit.clone()
                }
            }),
        }
    }
}

/// What a closure or a handler instance holds, in its cells' order: values with their types,
/// then capabilities, by name.
pub(crate) type Cells = (Vec<(Name, Type)>, Vec<Name>);

/// A body to compile: a function or predicate of the package, a lambda, or an operation of a
/// handler.
pub(crate) struct Body<'p> {
    /// For a closure or a handler's operation, whose first parameter is the closure or the
    /// instance: the values it holds, then the capabilities, by name, in its cells' order.
    pub(crate) this: Option<Cells>,
    /// The constraint of each type parameter of a function; its `params` end with the type
    /// descriptors of the constrained ones (`spec/20 §7`).
    pub(crate) constraints: Vec<Constraint>,
    /// Which of `params` the body owns (ADR 0072 D2): it releases them at the end of its
    /// scope, or moves them at their last use. The others, and any past its end, are borrowed.
    pub(crate) owned: Vec<bool>,
    /// The parameters.
    pub(crate) params: Vec<(Name, Type)>,
    /// The capabilities a function takes after its parameters, by name (`spec/16 §4` rule 4).
    pub(crate) uses: Vec<Name>,
    /// The result type, when it is declared.
    pub(crate) result: Option<Type>,
    /// Whether its integer arithmetic is exact, with no machine-width shortcut: where the
    /// verifier makes no `overflow` obligation (#180).
    pub(crate) exact: bool,
    /// The body.
    pub(crate) body: &'p Expr,
}

struct Lowerer<'e, 'p, 'b> {
    emitter: &'e mut Emitter<'p>,
    builder: FunctionBuilder<'b>,
    /// The bindings that other words borrow from: the scrutinees of `case`, the tuples of
    /// `let (...)` and the lists that loops visit. Their values are never moved (#30).
    pinned: BTreeSet<String>,
    /// The block that returns the status 2 of a stopped task at once (`spec/19 §6` rule 4), for
    /// an abort found where the words in scope are not at hand (U6003, U6004, U6005 and the
    /// faults of the prelude): the run, or the task, ends, and the task's heap with it. A
    /// cancellation, which a task survives inside a `within`, takes the failure path with
    /// [`STOPPED`] instead.
    stop: Block,
    /// Whether the code being lowered computes integers exactly (`Body::exact`): a fixed-width
    /// type there is not a proof that a value fits it.
    exact: bool,
}

/// Compiles `body` into `builder`'s function.
pub(crate) fn function<'p>(
    emitter: &mut Emitter<'p>,
    mut builder: FunctionBuilder<'_>,
    body: &Body<'p>,
    config: TargetFrontendConfig,
) -> Result<(), CodegenError> {
    let block = builder.create_block();
    builder.append_block_params_for_function_params(block);
    builder.switch_to_block(block);
    builder.seal_block(block);
    let mut params = builder.block_params(block).to_vec().into_iter();
    let stop = builder.create_block();
    let mut pinned = BTreeSet::new();
    borrowed_from(body.body, &mut pinned);
    let mut lowerer = Lowerer {
        emitter,
        builder,
        pinned,
        stop,
        exact: body.exact,
    };
    let mut env = Env::new();
    if let Some((values, caps)) = &body.this {
        let this = params
            .next()
            .ok_or_else(|| internal("a closure lost its parameter"))?;
        for (index, (name, ty)) in values.iter().enumerate() {
            kind(ty)?;
            let value = lowerer.field(this, index)?;
            env.insert(name.clone(), (value, ty.clone()));
        }
        for (index, name) in caps.iter().enumerate() {
            let value = lowerer.field(this, values.len() + index)?;
            env.insert(cap_key(name), (value, unknown()));
        }
    }
    let mut scope = Scope::new();
    for (index, (name, ty)) in body.params.iter().enumerate() {
        let parameter = kind(ty)?;
        let value = params
            .next()
            .ok_or_else(|| internal("a body lost a parameter"))?;
        env.insert(name.clone(), (value, ty.clone()));
        if body.owned.get(index).copied().unwrap_or(false) {
            scope.push((value, parameter));
        }
    }
    for name in &body.uses {
        let value = params
            .next()
            .ok_or_else(|| internal("a body lost a capability"))?;
        env.insert(cap_key(name), (value, unknown()));
    }
    if let Some(result) = &body.result {
        kind(result)?;
    }
    // Every body returns its value and whether it failed, the value being then the failure.
    let failed = lowerer.join();
    let catch = Catch {
        block: failed,
        outer: Scope::new(),
        base: 0,
        exit: None,
    };
    if let Some(result) = lowerer.expr(body.body, &mut env, &mut scope, &catch)? {
        let ok = lowerer.builder.ins().iconst(types::I64, 0);
        lowerer.builder.ins().return_(&[result, ok]);
    }
    // A failure returns status 1; a stop, which reaches here as [`STOPPED`] once every frame of
    // the body has released its words, returns status 2.
    lowerer.builder.switch_to_block(failed);
    let failure = lowerer.param(failed)?;
    let stopped = lowerer.stopped(failure);
    let one = lowerer.builder.ins().iconst(types::I64, 1);
    let status = lowerer.builder.ins().iadd(one, stopped);
    lowerer.builder.ins().return_(&[failure, status]);
    lowerer.builder.switch_to_block(stop);
    let word = lowerer.builder.ins().iconst(types::I64, STOPPED);
    let two = lowerer.builder.ins().iconst(types::I64, 2);
    lowerer.builder.ins().return_(&[word, two]);
    lowerer.builder.seal_all_blocks();
    lowerer.builder.finalize(config);
    Ok(())
}

impl<'p> Lowerer<'_, 'p, '_> {
    /// The owned word of `expr`, or `None` when its block ends without a value (an unreachable
    /// branch). `scope` holds the words to release at the end of each path, and a failure goes
    /// to `catch`.
    fn expr(
        &mut self,
        expr: &'p Expr,
        env: &mut Env,
        scope: &mut Scope,
        catch: &Catch,
    ) -> Result<Option<Value>, CodegenError> {
        let mut expr = expr;
        loop {
            match expr {
                // `let (x, ...) = t in e` (`spec/17 §4`): the components are borrowed from the
                // tuple, which its own binding keeps alive until the end of the scope.
                Expr::LetTuple {
                    names, value, body, ..
                } => {
                    let (tuple, _) = self.atom(value, env)?;
                    for (index, (name, ty)) in names.iter().enumerate() {
                        let ty = range::unrefined(ty);
                        kind(&ty)?;
                        let component = self.field(tuple, index)?;
                        env.insert(name.clone(), (component, ty));
                    }
                    expr = body;
                }
                Expr::Loop { .. } => return self.lower_loop(expr, env, scope, catch),
                Expr::Together { .. } | Expr::TogetherFor { .. } | Expr::FirstOf { .. } => {
                    return self.together(expr, env, scope, catch);
                }
                Expr::Tasks { bound, body, span } => {
                    return self.tasks(bound.as_ref(), body, *span, env, scope, catch);
                }
                Expr::Start { task, rights, span } => {
                    // ADR 0123 rule 4.2: the runtime moves the rights that the captures reach,
                    // looking only when one of them can hold a right.
                    return self.start(task, !rights.is_empty(), *span, env, scope, catch);
                }
                Expr::Quiet { body, timing, .. } => {
                    return self.quiet(body, *timing, env, scope, catch);
                }
                Expr::Atom { atom, .. } => {
                    let (value, kind) = self.atom(atom, env)?;
                    // A binding the scope owns moves out as the result; another word is
                    // retained, and the scope released (#30).
                    // An alias `let b = a` that does not move shares `a`'s word, with its own
                    // reference: one entry moves out, and any other is released.
                    let mut moved = matches!(atom, Atom::Var(_))
                        .then(|| scope.iter().rposition(|&(owned, _)| owned == value))
                        .flatten();
                    if moved.is_none() {
                        self.retain(value, kind);
                    }
                    for (position, &(owned, kind)) in scope.iter().enumerate().rev() {
                        if moved == Some(position) {
                            moved = None;
                            continue;
                        }
                        self.release(owned, kind);
                    }
                    return Ok(Some(value));
                }
                Expr::Let {
                    name,
                    ty,
                    value,
                    body,
                    written,
                    ..
                } => {
                    // A written type is an obligation; another binding's refinements are not
                    // proven of every value (`range`).
                    let ty = &if *written {
                        ty.clone()
                    } else {
                        range::unrefined(ty)
                    };
                    let kind = kind(ty)?;
                    let bound = match value.as_ref() {
                        Bound::Rhs(rhs) => self.rhs(rhs, ty, env, scope, catch, body)?,
                        Bound::Expr(inner) => {
                            // A binding the nested expression uses for the last time moves into
                            // its scope, so a branch can update it in place (#30).
                            let mut owned = self.handed_down(inner, body, env, scope, catch);
                            let nested = catch.nested(scope);
                            match self.expr(inner, &mut env.clone(), &mut owned, &nested)? {
                                Some(value) => value,
                                None => return Ok(None),
                            }
                        }
                    };
                    env.insert(name.clone(), (bound, ty.clone()));
                    scope.push((bound, kind));
                    expr = body;
                }
                Expr::If {
                    cond,
                    then,
                    otherwise,
                } => {
                    let (condition, _) = self.atom(cond, env)?;
                    let then_block = self.builder.create_block();
                    let else_block = self.builder.create_block();
                    let join = self.join();
                    self.builder
                        .ins()
                        .brif(condition, then_block, &[], else_block, &[]);
                    let mut reached = false;
                    for (block, branch) in [(then_block, then), (else_block, otherwise)] {
                        self.builder.switch_to_block(block);
                        reached |= self.branch(branch, env, scope, join, catch)?;
                    }
                    return self.joined(join, reached);
                }
                Expr::Case {
                    scrutinee,
                    arms,
                    otherwise,
                } => {
                    let (value, _) = self.atom(scrutinee, env)?;
                    let ty = self.atom_type(scrutinee, env);
                    let tag = self.tag(value)?;
                    let join = self.join();
                    let mut reached = false;
                    for arm in arms {
                        let arm_block = self.builder.create_block();
                        let next = self.builder.create_block();
                        let index = i64::try_from(arm.ctor.index).map_err(crate::internal)?;
                        let matches = self.builder.ins().icmp_imm_s(IntCC::Equal, tag, index);
                        self.builder.ins().brif(matches, arm_block, &[], next, &[]);
                        self.builder.switch_to_block(arm_block);
                        let mut inner = env.clone();
                        for (position, bind) in arm.binds.iter().enumerate() {
                            if let Some(name) = bind {
                                let field_ty =
                                    range::unrefined(&self.field_type(&ty, &arm.ctor, position));
                                kind(&field_ty)?;
                                let field = self.field(value, position)?;
                                inner.insert(name.clone(), (field, field_ty));
                            }
                        }
                        reached |= self.branch_in(&arm.body, &mut inner, scope, join, catch)?;
                        self.builder.switch_to_block(next);
                    }
                    match otherwise {
                        Some(otherwise) => {
                            reached |= self.branch(otherwise, env, scope, join, catch)?;
                        }
                        // The checker proves the arms exhaustive.
                        None => {
                            self.builder.ins().trap(TRAP);
                        }
                    }
                    return self.joined(join, reached);
                }
                Expr::Unreachable { .. } => {
                    // The `exhaustive` obligation proves this branch is never taken.
                    self.builder.ins().trap(TRAP);
                    return Ok(None);
                }
                Expr::Fail { ctor, args, .. } => {
                    let failure = self.construct(ctor, args, env)?;
                    self.fail(failure, scope, catch);
                    return Ok(None);
                }
                // `skip` or `stop`: the state, owned, goes to the loop's block for it, and the
                // body's words are released (`spec/26 §3`).
                Expr::Exit { tag, state, .. } => {
                    let exit = catch
                        .exit
                        .clone()
                        .ok_or_else(|| internal("an exit outside a loop"))?;
                    let mut values: Vec<cranelift_codegen::ir::BlockArg> = Vec::new();
                    for (atom, &kind) in state.iter().zip(&exit.kinds) {
                        let (value, _) = self.atom(atom, env)?;
                        self.retain(value, kind);
                        values.push(value.into());
                    }
                    for &(owned, kind) in scope.iter().skip(exit.base).rev() {
                        self.release(owned, kind);
                    }
                    for &(owned, kind) in exit.outer.iter().rev() {
                        self.release(owned, kind);
                    }
                    let block = match tag {
                        ExitTag::Next => exit.next,
                        ExitTag::Stop => exit.stop,
                    };
                    self.builder.ins().jump(block, &values);
                    return Ok(None);
                }
                Expr::Handle {
                    body,
                    error,
                    handler,
                } => {
                    // The body's value is the clause's; its failure goes to the handler, which
                    // owns it.
                    let join = self.join();
                    let handling = self.join();
                    let inner = Catch {
                        block: handling,
                        outer: Scope::new(),
                        base: scope.len(),
                        exit: catch.exit.clone(),
                    };
                    let mut reached = self.branch(body, env, scope, join, &inner)?;
                    self.builder.switch_to_block(handling);
                    let failure = self.param(handling)?;
                    // A stop is no failure to handle: it goes on, out of this frame too.
                    self.pass_stop(failure, scope, catch);
                    let mut handler_env = env.clone();
                    handler_env.insert(error.clone(), (failure, unknown()));
                    let mut handler_scope = scope.clone();
                    handler_scope.push((failure, Kind::Any));
                    reached |=
                        self.branch_in(handler, &mut handler_env, &handler_scope, join, catch)?;
                    return self.joined(join, reached);
                }
                Expr::Attempt { body, .. } => {
                    return self.attempt(body, env, scope, catch).map(Some);
                }
                Expr::Within {
                    deadline,
                    body,
                    timeout,
                    wrap,
                    ..
                } => {
                    return self.within(
                        deadline,
                        body,
                        timeout,
                        wrap.as_deref(),
                        env,
                        scope,
                        catch,
                    );
                }
                Expr::With { cap, source, body } => {
                    let instance = match source {
                        // E-Rebind shares the instance.
                        Source::Rebind(other) => {
                            env.get(&cap_key(other))
                                .ok_or_else(|| internal(format!("no capability `@{other}`")))?
                                .0
                        }
                        Source::Handler {
                            handler,
                            args,
                            given,
                            ..
                        } => {
                            if *given {
                                return Err(unsupported("examples"));
                            }
                            let instance = self.install(handler, args, env)?;
                            scope.push((instance, Kind::Fn));
                            instance
                        }
                    };
                    env.insert(cap_key(cap), (instance, unknown()));
                    expr = body;
                }
            }
        }
    }

    /// `quiet e` (S-39, `spec/26 §6` rule 4): `e` between the runtime's count of the task's
    /// `quiet` expressions and its restoration, on every way out of `e`. A failure or a stop of
    /// the task leaves `e` through a catch of its own, which restores the count, then goes on to
    /// `catch`; `e`'s words are released on the way, as `catch` would. An `exit` of an
    /// enclosing loop, which crosses a `tasks` statement in `e`, leaves through blocks of their
    /// own, which restore the count and go on to the loop's: the exit has released its words
    /// already.
    fn quiet(
        &mut self,
        body: &'p Expr,
        timing: bool,
        env: &mut Env,
        scope: &mut Scope,
        catch: &Catch,
    ) -> Result<Option<Value>, CodegenError> {
        // A `quiet` that a timing floor alone marked keeps its operations' events (S-42).
        let hush = self.builder.ins().iconst(types::I64, i64::from(!timing));
        self.runtime("una_rt_quiet_enter", &[hush]);
        let leaving = self.join();
        let crossing = catch.exit.as_ref().map(|outer| {
            let next = self.builder.create_block();
            let stop = self.builder.create_block();
            for _ in &outer.kinds {
                self.builder.append_block_param(next, types::I64);
                self.builder.append_block_param(stop, types::I64);
            }
            (outer.clone(), next, stop)
        });
        // The body's failures release what `catch` would release of `scope`, and the restoring
        // block then what it would release of the enclosing expressions.
        let inner = Catch {
            block: leaving,
            outer: Scope::new(),
            base: catch.base,
            exit: crossing.as_ref().map(|(outer, next, stop)| Exit {
                next: *next,
                stop: *stop,
                ..outer.clone()
            }),
        };
        let done = self.join();
        let reached = match self.expr(body, env, scope, &inner)? {
            Some(value) => {
                self.runtime("una_rt_quiet_leave", &[hush]);
                self.builder.ins().jump(done, &[value.into()]);
                true
            }
            None => false,
        };
        self.builder.switch_to_block(leaving);
        let failure = self.param(leaving)?;
        self.runtime("una_rt_quiet_leave", &[hush]);
        self.fail(failure, &Scope::new(), catch);
        if let Some((outer, next, stop)) = crossing {
            for (block, target) in [(next, outer.next), (stop, outer.stop)] {
                self.builder.switch_to_block(block);
                let state: Vec<Value> = self.builder.block_params(block).to_vec();
                self.runtime("una_rt_quiet_leave", &[hush]);
                let args: Vec<cranelift_codegen::ir::BlockArg> =
                    state.iter().map(|&value| value.into()).collect();
                self.builder.ins().jump(target, &args);
            }
        }
        self.joined(done, reached)
    }

    /// A join block with one parameter, the value of the construct.
    fn join(&mut self) -> Block {
        let join = self.builder.create_block();
        self.builder.append_block_param(join, types::I64);
        join
    }

    /// Lowers `branch` in the current block, with its own copies of `env` and `scope`, and
    /// jumps to `join` with its value; whether it reaches `join`.
    fn branch(
        &mut self,
        branch: &'p Expr,
        env: &Env,
        scope: &Scope,
        join: Block,
        catch: &Catch,
    ) -> Result<bool, CodegenError> {
        self.branch_in(branch, &mut env.clone(), scope, join, catch)
    }

    fn branch_in(
        &mut self,
        branch: &'p Expr,
        env: &mut Env,
        scope: &Scope,
        join: Block,
        catch: &Catch,
    ) -> Result<bool, CodegenError> {
        match self.expr(branch, env, &mut scope.clone(), catch)? {
            Some(value) => {
                self.builder.ins().jump(join, &[value.into()]);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Continues at `join`, whose parameter is the construct's value, when a branch reaches it.
    fn joined(&mut self, join: Block, reached: bool) -> Result<Option<Value>, CodegenError> {
        self.builder.switch_to_block(join);
        if !reached {
            // No branch returns: the join is never reached.
            self.builder.ins().trap(TRAP);
            return Ok(None);
        }
        self.param(join).map(Some)
    }

    /// A `for ever` loop with its `on stop` (`spec/26 §5`): as [`Self::lower_until`], without
    /// a condition. `on stop` runs on the final state, its value the loop's; after a failure of
    /// the body it runs on a copy of the state from before the loop, which the loop keeps for
    /// it, and the failure then goes on to `catch` (ADR 0116 rule 10).
    fn lower_ever(
        &mut self,
        expr: &'p Expr,
        env: &Env,
        scope: &mut Scope,
        catch: &Catch,
    ) -> Result<Option<Value>, CodegenError> {
        // The runtime starts its threads before `main` (#1145).
        self.emitter.threads = true;
        let Expr::Loop {
            source,
            var,
            state,
            init,
            body,
            on_stop,
            span,
        } = expr
        else {
            return Err(internal("a loop that is not one"));
        };
        // `within d` (`spec/26 §7`): evaluated once, before the first iteration; the runtime
        // arms the deadline at each iteration and drops it when the loop ends.
        let within = match source.as_ref() {
            LoopSource::Ever { within } => within.as_ref(),
            _ => None,
        };
        if let Some((deadline, _)) = within {
            let (word, _) = self.atom(deadline, env)?;
            self.runtime("una_rt_deadline_enter", &[word]);
        }
        let types: Vec<Type> = state.iter().map(|(_, ty)| range::unrefined(ty)).collect();
        let kinds: Vec<Kind> = types.iter().map(kind).collect::<Result<_, _>>()?;
        let mut initial: Vec<cranelift_codegen::ir::BlockArg> = Vec::new();
        let mut saved: Vec<Value> = Vec::new();
        for (atom, &kind) in init.iter().zip(&kinds) {
            let (value, _) = self.atom(atom, env)?;
            match self.moving(atom, env, scope, catch, body) {
                Some(position) => {
                    scope.remove(position);
                }
                None => self.retain(value, kind),
            }
            // The copy that `on stop` runs on after a failure.
            if on_stop.is_some() {
                self.retain(value, kind);
                saved.push(value);
            }
            initial.push(value.into());
        }
        let site = self.site(*span);
        let head = self.builder.create_block();
        let following = self.builder.create_block();
        let exit = self.builder.create_block();
        let failing = self.join();
        for _ in &kinds {
            self.builder.append_block_param(head, types::I64);
            self.builder.append_block_param(following, types::I64);
            self.builder.append_block_param(exit, types::I64);
        }
        // The iterations this instance of the loop has started, from 0 at each entry: the
        // budget of `--stop-after-iterations` is per loop instance (ADR 0116 rule 19, #1472).
        let count = self.builder.declare_var(types::I64);
        let none_started = self.builder.ins().iconst(types::I64, 0);
        self.builder.def_var(count, none_started);
        self.builder.ins().jump(head, &initial);

        // An iteration starts: a host stop, or the end of this instance's budget, ends the
        // loop there (ADR 0116 rules 14 and 19), then the deadline's budget, then the body,
        // which owns the state, with its exits.
        self.builder.switch_to_block(head);
        let current = self.builder.block_params(head).to_vec();
        let so_far = self.builder.use_var(count);
        let host_stop = self.call("una_rt_ever_start", &[so_far])?;
        let started = self.builder.create_block();
        let current_arguments: Vec<cranelift_codegen::ir::BlockArg> =
            current.iter().map(|&v| v.into()).collect();
        self.builder
            .ins()
            .brif(host_stop, exit, &current_arguments, started, &[]);
        self.builder.switch_to_block(started);
        let one_more = self.builder.ins().iadd_imm_s(so_far, 1);
        self.builder.def_var(count, one_more);
        if within.is_some() {
            self.runtime("una_rt_deadline_iteration", &[]);
        }
        let mut inner = env.clone();
        for ((name, _), (&value, ty)) in state.iter().zip(current.iter().zip(&types)) {
            inner.insert(name.clone(), (value, ty.clone()));
        }
        let unit_value = self.builder.ins().iconst(types::I64, 0);
        inner.insert(var.0.clone(), (unit_value, range::unrefined(&var.1)));
        let exits = Exit {
            next: following,
            stop: exit,
            kinds: kinds.clone(),
            outer: Scope::new(),
            base: 0,
        };
        // A failure of the body leaves the body (ADR 0116 rule 14), then goes to `on stop`
        // first, when there is one.
        let nested = Catch {
            block: failing,
            outer: Scope::new(),
            base: 0,
            exit: Some(exits),
        };
        let mut owned: Scope = current.iter().copied().zip(kinds.iter().copied()).collect();
        // The iteration's budget: a stop there leaves as a failure of the body would.
        let budget = self.call("una_rt_budget", &[site])?;
        self.stop_or_fail(budget, &owned, &nested);
        if let Some(tuple) = self.expr(body, &mut inner, &mut owned, &nested)? {
            let mut next: Vec<cranelift_codegen::ir::BlockArg> = Vec::new();
            for (index, &kind) in kinds.iter().enumerate() {
                let value = self.field(tuple, index)?;
                self.retain(value, kind);
                next.push(value.into());
            }
            self.release(tuple, Kind::Tuple);
            self.builder.ins().jump(following, &next);
        }

        // The next iteration, from the end of the body or a `skip`.
        self.builder.switch_to_block(following);
        let next: Vec<cranelift_codegen::ir::BlockArg> = self
            .builder
            .block_params(following)
            .iter()
            .map(|&v| v.into())
            .collect();
        self.builder.ins().jump(head, &next);

        // After a failure: out of the body, `on stop` on the saved state, then the failure goes
        // on.
        self.builder.switch_to_block(failing);
        let failure = self.param(failing)?;
        self.runtime("una_rt_ever_end", &[]);
        if within.is_some() {
            self.runtime("una_rt_deadline_leave", &[]);
        }
        match on_stop {
            Some(on_stop) => {
                // A stop of the task: a cancellation, as a `within` around the loop whose
                // deadline passed, ends the loop as after a failure, with `on stop` shielded
                // from it (ADR 0116 rule 15, ADR 0117 rule 5); an abort goes on at once, the
                // saved state released.
                let stopped = self.stopped(failure);
                let stopping = self.builder.create_block();
                let failing_on = self.builder.create_block();
                self.builder
                    .ins()
                    .brif(stopped, stopping, &[], failing_on, &[]);
                self.builder.switch_to_block(failing_on);
                let result = self.lower_on_stop(on_stop, env, &saved, &kinds, catch, scope)?;
                if let Some(result) = result {
                    self.release(result, Kind::Tuple);
                    self.fail(failure, scope, catch);
                }
                self.builder.switch_to_block(stopping);
                let polled = self.call("una_rt_poll", &[])?;
                let aborted = self.call("una_rt_stopped", &[])?;
                let cancelling = self.builder.ins().icmp_imm_s(IntCC::NotEqual, polled, 0);
                let healthy = self.builder.ins().icmp_imm_s(IntCC::Equal, aborted, 0);
                let run = self.builder.ins().band(cancelling, healthy);
                let run = self.flag(run);
                let running = self.builder.create_block();
                let skipping = self.builder.create_block();
                self.builder.ins().brif(run, running, &[], skipping, &[]);
                self.builder.switch_to_block(skipping);
                for (&value, &kind) in saved.iter().zip(&kinds) {
                    self.release(value, kind);
                }
                self.fail(failure, scope, catch);
                self.builder.switch_to_block(running);
                self.runtime("una_rt_operation_begin", &[]);
                // The shield ends on every way out of `on stop`: a failure out of it, an abort
                // today, ends it before going on to `catch` with the loop's scope.
                let unshield = self.join();
                let shielded = Catch {
                    block: unshield,
                    outer: Scope::new(),
                    base: 0,
                    exit: None,
                };
                let result =
                    self.lower_on_stop(on_stop, env, &saved, &kinds, &shielded, &Scope::new())?;
                if let Some(result) = result {
                    self.release(result, Kind::Tuple);
                    self.runtime("una_rt_operation_end", &[]);
                    self.fail(failure, scope, catch);
                }
                self.builder.switch_to_block(unshield);
                let out = self.param(unshield)?;
                self.runtime("una_rt_operation_end", &[]);
                self.fail(out, scope, catch);
            }
            None => self.fail(failure, scope, catch),
        }

        // The loop's value: the final state, through `on stop` when there is one, out of the
        // body.
        self.builder.switch_to_block(exit);
        let finals = self.builder.block_params(exit).to_vec();
        if within.is_some() {
            self.runtime("una_rt_deadline_leave", &[]);
        }
        self.runtime("una_rt_ever_end", &[]);
        let tuple = match on_stop {
            Some(on_stop) => {
                for (&value, &kind) in saved.iter().zip(&kinds) {
                    self.release(value, kind);
                }
                match self.lower_on_stop(on_stop, env, &finals, &kinds, catch, scope)? {
                    Some(tuple) => tuple,
                    None => return Ok(None),
                }
            }
            None => {
                let zero = self.builder.ins().iconst(types::I64, 0);
                let tuple = self.call("una_rt_data", &[zero])?;
                for (&value, &kind) in finals.iter().zip(&kinds) {
                    self.runtime("una_rt_push", &[tuple, value]);
                    self.release(value, kind);
                }
                tuple
            }
        };
        for &(owned, kind) in scope.iter().rev() {
            self.release(owned, kind);
        }
        Ok(Some(tuple))
    }

    /// The `on stop` block on the owned state `values`: its tuple, owned, or `None` when it
    /// never ends normally. A failure in it goes to `catch` with the words of `scope`.
    fn lower_on_stop(
        &mut self,
        on_stop: &'p una_core::OnStop,
        env: &Env,
        values: &[Value],
        kinds: &[Kind],
        catch: &Catch,
        scope: &Scope,
    ) -> Result<Option<Value>, CodegenError> {
        let mut inner = env.clone();
        for ((name, ty), &value) in on_stop.state.iter().zip(values) {
            inner.insert(name.clone(), (value, range::unrefined(ty)));
        }
        let mut owned: Scope = values.iter().copied().zip(kinds.iter().copied()).collect();
        let mut nested = catch.nested(scope);
        nested.exit = None;
        self.expr(&on_stop.body, &mut inner, &mut owned, &nested)
    }

    /// A `for until` loop (`spec/26 §4`): as [`Self::lower_loop`], without an item, and with
    /// its condition evaluated before each iteration, with the state it owns borrowed.
    fn lower_until(
        &mut self,
        expr: &'p Expr,
        env: &Env,
        scope: &mut Scope,
        catch: &Catch,
    ) -> Result<Option<Value>, CodegenError> {
        let Expr::Loop {
            source,
            var,
            state,
            init,
            body,
            span,
            ..
        } = expr
        else {
            return Err(internal("a loop that is not one"));
        };
        let LoopSource::Until { cond, .. } = source.as_ref() else {
            return Err(internal("an `until` loop without its condition"));
        };
        let types: Vec<Type> = state.iter().map(|(_, ty)| range::unrefined(ty)).collect();
        let kinds: Vec<Kind> = types.iter().map(kind).collect::<Result<_, _>>()?;
        // The loop owns its initial state: a binding at its last use moves in, any other word
        // is retained (#30).
        let mut initial: Vec<cranelift_codegen::ir::BlockArg> = Vec::new();
        for (atom, &kind) in init.iter().zip(&kinds) {
            let (value, _) = self.atom(atom, env)?;
            match self.moving(atom, env, scope, catch, body) {
                Some(position) => {
                    scope.remove(position);
                }
                None => self.retain(value, kind),
            }
            initial.push(value.into());
        }
        let site = self.site(*span);
        let head = self.builder.create_block();
        let iteration = self.builder.create_block();
        let following = self.builder.create_block();
        let exit = self.builder.create_block();
        for _ in &kinds {
            self.builder.append_block_param(head, types::I64);
            self.builder.append_block_param(following, types::I64);
            self.builder.append_block_param(exit, types::I64);
        }
        self.builder.ins().jump(head, &initial);

        // The head: the condition, with the state borrowed; a failure releases the state.
        self.builder.switch_to_block(head);
        let current = self.builder.block_params(head).to_vec();
        let mut inner = env.clone();
        for ((name, _), (&value, ty)) in state.iter().zip(current.iter().zip(&types)) {
            inner.insert(name.clone(), (value, ty.clone()));
        }
        let owned_state: Scope = current.iter().copied().zip(kinds.iter().copied()).collect();
        let mut guarded = catch.nested(scope);
        guarded.outer.extend(owned_state.iter().copied());
        guarded.exit = None;
        let mut cond_env = inner.clone();
        let holds = self.expr(cond, &mut cond_env, &mut Scope::new(), &guarded)?;
        let state_arguments: Vec<cranelift_codegen::ir::BlockArg> =
            current.iter().map(|&v| v.into()).collect();
        if let Some(holds) = holds {
            self.builder
                .ins()
                .brif(holds, exit, &state_arguments, iteration, &[]);
        }

        // An iteration: its budget, then the body, which owns the state, with its exits. A stop
        // at the budget leaves as a failure of the body would, the state released.
        self.builder.switch_to_block(iteration);
        let unit_value = self.builder.ins().iconst(types::I64, 0);
        inner.insert(var.0.clone(), (unit_value, range::unrefined(&var.1)));
        let mut nested = catch.nested(scope);
        nested.exit = Some(Exit {
            next: following,
            stop: exit,
            kinds: kinds.clone(),
            outer: Scope::new(),
            base: 0,
        });
        let mut owned = owned_state;
        let stopping = self.call("una_rt_budget", &[site])?;
        self.stop_or_fail(stopping, &owned, &nested);
        if let Some(tuple) = self.expr(body, &mut inner, &mut owned, &nested)? {
            let mut next: Vec<cranelift_codegen::ir::BlockArg> = Vec::new();
            for (index, &kind) in kinds.iter().enumerate() {
                let value = self.field(tuple, index)?;
                self.retain(value, kind);
                next.push(value.into());
            }
            self.release(tuple, Kind::Tuple);
            self.builder.ins().jump(following, &next);
        }

        // The next iteration, from the end of the body or a `skip`.
        self.builder.switch_to_block(following);
        let next: Vec<cranelift_codegen::ir::BlockArg> = self
            .builder
            .block_params(following)
            .iter()
            .map(|&v| v.into())
            .collect();
        self.builder.ins().jump(head, &next);

        // The loop's value, which owns the final state, and the end of the scope.
        self.builder.switch_to_block(exit);
        let finals = self.builder.block_params(exit).to_vec();
        let zero = self.builder.ins().iconst(types::I64, 0);
        let tuple = self.call("una_rt_data", &[zero])?;
        for (&value, &kind) in finals.iter().zip(&kinds) {
            self.runtime("una_rt_push", &[tuple, value]);
            self.release(value, kind);
        }
        for &(owned, kind) in scope.iter().rev() {
            self.release(owned, kind);
        }
        Ok(Some(tuple))
    }

    /// A `for` loop (`spec/17 §4`): the loop owns the current values of its state, passes them
    /// to each iteration with the item, and takes the new values from the tuple the body
    /// returns. Iterations are not calls, so each checks the memory budget at the loop instead
    /// (ADR 0061 rule 2). The loop's value, the tuple of the final state, is the value of the
    /// expression whose scope is `scope`, which the loop releases at its end.
    fn lower_loop(
        &mut self,
        expr: &'p Expr,
        env: &Env,
        scope: &mut Scope,
        catch: &Catch,
    ) -> Result<Option<Value>, CodegenError> {
        let Expr::Loop {
            source,
            var,
            state,
            init,
            body,
            span,
            ..
        } = expr
        else {
            return Err(internal("a loop that is not one"));
        };
        // `for until` and `for ever` take their initial state themselves: taking it here too
        // would retain each word once more, and leak it (#1198).
        match source.as_ref() {
            LoopSource::Until { .. } => return self.lower_until(expr, env, scope, catch),
            LoopSource::Ever { .. } => return self.lower_ever(expr, env, scope, catch),
            LoopSource::List(_) | LoopSource::Range { .. } => {}
        }
        let types: Vec<Type> = state.iter().map(|(_, ty)| range::unrefined(ty)).collect();
        let kinds: Vec<Kind> = types.iter().map(kind).collect::<Result<_, _>>()?;
        // The loop owns its initial state: a binding at its last use moves in, any other word
        // is retained (#30).
        let mut initial = Vec::new();
        for (atom, &kind) in init.iter().zip(&kinds) {
            let (value, _) = self.atom(atom, env)?;
            match self.moving(atom, env, scope, catch, body) {
                Some(position) => {
                    scope.remove(position);
                }
                None => self.retain(value, kind),
            }
            initial.push(value);
        }
        let site = self.site(*span);
        // What the loop visits: the items of a list, by index, or the integers of a range, by
        // an owned cursor from the low bound up to the high bound, which the loop also owns:
        // the source is evaluated once, and the body may update the binding that names it
        // (`spec/17 §4`, #182).
        let (list, bound, start) = match source.as_ref() {
            LoopSource::List(list) => {
                let (list, _) = self.atom(list, env)?;
                let count = self.call("una_rt_count", &[list])?;
                let zero = self.builder.ins().iconst(types::I64, 0);
                (Some(list), count, zero)
            }
            LoopSource::Range { low, high, .. } => {
                let (low, _) = self.atom(low, env)?;
                let (high, _) = self.atom(high, env)?;
                self.retain(low, Kind::Int);
                self.retain(high, Kind::Int);
                (None, high, low)
            }
            LoopSource::Until { .. } | LoopSource::Ever { .. } => {
                return Err(internal("a `for until` or `for ever` lowered as a `for`"));
            }
        };
        let inclusive = matches!(
            source.as_ref(),
            LoopSource::Range {
                inclusive: true,
                ..
            }
        );
        let head = self.builder.create_block();
        let iteration = self.builder.create_block();
        let leave = self.builder.create_block();
        let exit = self.builder.create_block();
        self.builder.append_block_param(head, types::I64);
        for _ in &kinds {
            self.builder.append_block_param(head, types::I64);
            self.builder.append_block_param(exit, types::I64);
        }
        let mut arguments: Vec<cranelift_codegen::ir::BlockArg> = vec![start.into()];
        arguments.extend(
            initial
                .iter()
                .map(|&v| cranelift_codegen::ir::BlockArg::from(v)),
        );
        self.builder.ins().jump(head, &arguments);

        self.builder.switch_to_block(head);
        let params = self.builder.block_params(head).to_vec();
        let (Some(&cursor), current) = (params.first(), params.get(1..).unwrap_or(&[])) else {
            return Err(internal("a loop lost its cursor"));
        };
        let current = current.to_vec();
        let more = match list {
            Some(_) => {
                let flag = self
                    .builder
                    .ins()
                    .icmp(IntCC::SignedLessThan, cursor, bound);
                self.flag(flag)
            }
            None => {
                let cc = if inclusive {
                    IntCC::SignedLessThanOrEqual
                } else {
                    IntCC::SignedLessThan
                };
                self.compare_ints(cc, cursor, bound)?
            }
        };
        self.builder.ins().brif(more, iteration, &[], leave, &[]);

        // The loop ends: a range's cursor is released, and the state goes to the exit.
        self.builder.switch_to_block(leave);
        if list.is_none() {
            self.release(cursor, Kind::Int);
        }
        let state_arguments: Vec<cranelift_codegen::ir::BlockArg> =
            current.iter().map(|&v| v.into()).collect();
        self.builder.ins().jump(exit, &state_arguments);

        self.builder.switch_to_block(iteration);
        let item = match list {
            Some(list) => self.call("una_rt_field", &[list, cursor])?,
            None => cursor,
        };
        let mut inner = env.clone();
        inner.insert(var.0.clone(), (item, range::unrefined(&var.1)));
        for ((name, _), (&value, ty)) in state.iter().zip(current.iter().zip(&types)) {
            inner.insert(name.clone(), (value, ty.clone()));
        }
        // The iteration owns the current state, which its body releases at its end or moves to
        // its last use, so `acc = acc.append(item: x)` updates the list in place (#30). A
        // failure in the body releases the state with the body's scope, and the cursor; a
        // `skip` or a `stop` releases the body's scope only (`spec/26 §3`).
        let following = self.builder.create_block();
        let stopping = self.builder.create_block();
        for _ in &kinds {
            self.builder.append_block_param(following, types::I64);
            self.builder.append_block_param(stopping, types::I64);
        }
        let mut nested = catch.nested(scope);
        if list.is_none() {
            nested.outer.push((bound, Kind::Int));
            nested.outer.push((cursor, Kind::Int));
        }
        nested.exit = Some(Exit {
            next: following,
            stop: stopping,
            kinds: kinds.clone(),
            outer: Scope::new(),
            base: 0,
        });
        let mut owned: Scope = current.iter().copied().zip(kinds.iter().copied()).collect();
        // The iteration's budget: a stop there leaves as a failure of the body would, the state
        // and a range's cursor released.
        let budget = self.call("una_rt_budget", &[site])?;
        self.stop_or_fail(budget, &owned, &nested);
        if let Some(tuple) = self.expr(body, &mut inner, &mut owned, &nested)? {
            let mut next: Vec<cranelift_codegen::ir::BlockArg> = Vec::new();
            for (index, &kind) in kinds.iter().enumerate() {
                let value = self.field(tuple, index)?;
                self.retain(value, kind);
                next.push(value.into());
            }
            self.release(tuple, Kind::Tuple);
            self.builder.ins().jump(following, &next);
        }

        // `stop`: a range's cursor is released, and the state goes to the exit.
        self.builder.switch_to_block(stopping);
        let stopped: Vec<cranelift_codegen::ir::BlockArg> = self
            .builder
            .block_params(stopping)
            .iter()
            .map(|&v| v.into())
            .collect();
        if list.is_none() {
            self.release(cursor, Kind::Int);
        }
        self.builder.ins().jump(exit, &stopped);

        // The next iteration, from the end of the body or a `skip`.
        self.builder.switch_to_block(following);
        {
            let next = self.builder.block_params(following).to_vec();
            let mut next_arguments: Vec<cranelift_codegen::ir::BlockArg> =
                next.iter().map(|&v| v.into()).collect();
            match list {
                Some(_) => {
                    let step = self.builder.ins().iadd_imm_s(cursor, 1);
                    next_arguments.insert(0, step.into());
                    self.builder.ins().jump(head, &next_arguments);
                }
                None => {
                    // The successor of the upper bound is never computed, so a range up to the
                    // greatest value of a type stops there, as the interpreter does.
                    let last = self.compare_ints(IntCC::Equal, cursor, bound)?;
                    let stop = self.builder.create_block();
                    let step = self.builder.create_block();
                    self.builder.ins().brif(last, stop, &[], step, &[]);
                    self.builder.switch_to_block(stop);
                    self.release(cursor, Kind::Int);
                    let final_arguments: Vec<cranelift_codegen::ir::BlockArg> =
                        next.iter().map(|&v| v.into()).collect();
                    self.builder.ins().jump(exit, &final_arguments);
                    self.builder.switch_to_block(step);
                    let one = self.builder.ins().iconst(types::I64, (1 << 1) | 1);
                    let successor = self.arithmetic(Prim::Add, cursor, one)?;
                    self.release(cursor, Kind::Int);
                    next_arguments.insert(0, successor.into());
                    self.builder.ins().jump(head, &next_arguments);
                }
            }
        }

        // The loop's value, which owns the final state, and the end of the scope.
        self.builder.switch_to_block(exit);
        if list.is_none() {
            self.release(bound, Kind::Int);
        }
        let finals = self.builder.block_params(exit).to_vec();
        let zero = self.builder.ins().iconst(types::I64, 0);
        let tuple = self.call("una_rt_data", &[zero])?;
        for (&value, &kind) in finals.iter().zip(&kinds) {
            self.runtime("una_rt_push", &[tuple, value]);
            self.release(value, kind);
        }
        for &(owned, kind) in scope.iter().rev() {
            self.release(owned, kind);
        }
        Ok(Some(tuple))
    }

    /// The position in `scope` of the binding `atom` names, when this is its last use, before
    /// `rest`, and its value can move: the scope owns it since `catch` was installed, and no
    /// other word borrows from it (#30).
    fn moving(
        &self,
        atom: &Atom,
        env: &Env,
        scope: &Scope,
        catch: &Catch,
        rest: &Expr,
    ) -> Option<usize> {
        let Atom::Var(name) = atom else {
            return None;
        };
        if self.pinned.contains(name) {
            return None;
        }
        let (value, _) = env.get(name.as_str())?;
        let position = scope
            .iter()
            .enumerate()
            .skip(catch.base)
            .find(|(_, (owned, _))| owned == value)
            .map(|(position, _)| position)?;
        let mut later = BTreeSet::new();
        vars(rest, &mut later);
        self.unheld(*value, env, &later, &[]).then_some(position)
    }

    /// Whether no binding holds `word` after this point: every name bound to it, the one in
    /// use and its aliases, is dead in `later`, not pinned, and not named by `args` (#181). A
    /// `let` of another binding at its last use shares the word, so a dead name and a live
    /// alias can hold the same word.
    fn unheld(&self, word: Value, env: &Env, later: &BTreeSet<String>, args: &[Atom]) -> bool {
        env.iter()
            .filter(|(_, (value, _))| *value == word)
            .all(|(name, _)| {
                !later.contains(name)
                    && !self.pinned.contains(name)
                    && !args
                        .iter()
                        .any(|arg| matches!(arg, Atom::Var(n) if n == name))
            })
    }

    /// The entries of `scope` that `inner`, a nested expression, uses for the last time before
    /// `rest`, removed from `scope`: the nested expression's scope starts with them (#30).
    fn handed_down(
        &self,
        inner: &Expr,
        rest: &Expr,
        env: &Env,
        scope: &mut Scope,
        catch: &Catch,
    ) -> Scope {
        let mut used = BTreeSet::new();
        vars(inner, &mut used);
        let mut later = BTreeSet::new();
        vars(rest, &mut later);
        let mut handed = Scope::new();
        let mut position = scope.len();
        while position > catch.base {
            position -= 1;
            let Some(&(owned, kind)) = scope.get(position) else {
                break;
            };
            // Every binding of the word: an alias that did not move shares it.
            let moves = env
                .iter()
                .any(|(name, (value, _))| *value == owned && used.contains(name))
                && self.unheld(owned, env, &later, &[]);
            if moves {
                scope.remove(position);
                handed.push((owned, kind));
            }
        }
        handed
    }

    /// A binding of type `ty` that dies before `rest`, which `args` does not name, and whose
    /// value can move: its object and its position in `scope`.
    fn reusable(
        &self,
        ty: &Type,
        args: &[Atom],
        env: &Env,
        scope: &Scope,
        catch: &Catch,
        rest: &Expr,
    ) -> Option<(Value, usize)> {
        let Type::Base {
            base: Base::Named { module, name, .. },
            ..
        } = ty
        else {
            return None;
        };
        let mut later = BTreeSet::new();
        vars(rest, &mut later);
        scope
            .iter()
            .enumerate()
            .skip(catch.base)
            .rev()
            .find_map(|(position, &(owned, _))| {
                let (_, (_, binding_ty)) = env.iter().find(|(_, (value, _))| *value == owned)?;
                let same_kind = matches!(
                    binding_ty,
                    Type::Base { base: Base::Named { module: m, name: n, .. }, .. }
                        if m == module && n == name
                );
                (same_kind && self.unheld(owned, env, &later, args)).then_some((owned, position))
            })
    }

    /// A new closure of the lambda `params` to `body`, which captures the values of `env` that
    /// `body` uses and every capability in scope (`spec/11 §4`).
    fn closure(
        &mut self,
        params: &'p [(Name, Type)],
        body: &'p Expr,
        env: &Env,
    ) -> Result<Value, CodegenError> {
        Ok(self.counted_closure(params, body, env)?.0)
    }

    /// [`Lowerer::closure`], and how many values it captures, which come before the instances of
    /// its capabilities in its environment.
    fn counted_closure(
        &mut self,
        params: &'p [(Name, Type)],
        body: &'p Expr,
        env: &Env,
    ) -> Result<(Value, usize), CodegenError> {
        let mut names = BTreeSet::new();
        vars(body, &mut names);
        // The type descriptors in scope too: the lambda's body may compare values of a type
        // parameter, or call a constrained function with one (`spec/20 §7`).
        names.extend(env.keys().filter(|key| key.starts_with('$')).cloned());
        let captures: Vec<(Name, Type)> = names
            .into_iter()
            .filter(|name| !name.starts_with('@'))
            .filter(|name| !params.iter().any(|(p, _)| p == name))
            .filter_map(|name| {
                let ty = env.get(&name)?.1.clone();
                Some((name, ty))
            })
            .collect();
        // A lambda keeps the capabilities in scope where it is made (E-Lam, `spec/11 §4`).
        let caps: Vec<Name> = env
            .keys()
            .filter_map(|key| key.strip_prefix('@'))
            .map(str::to_owned)
            .collect();
        let count = captures.len();
        let values: Vec<Value> = captures
            .iter()
            .map(|(name, _)| name.clone())
            .chain(caps.iter().map(|name| cap_key(name)))
            .filter_map(|name| env.get(&name).map(|(value, _)| *value))
            .collect();
        let id = self
            .emitter
            .lambda(params, body, captures, caps, self.exact)?;
        let code = self
            .emitter
            .module
            .declare_func_in_func(id, self.builder.func);
        let address = self.builder.ins().func_addr(types::I64, code);
        // The lambda takes the closure and `params`, as `Emitter::lambda` declares it: the
        // runtime checks this arity before it calls the code on a task's thread (#424).
        let arity = i64::try_from(params.len()).map_err(crate::internal)?;
        let arity = self.builder.ins().iconst(types::I64, arity);
        let closure = self.call("una_rt_closure", &[address, arity])?;
        for value in values {
            self.runtime("una_rt_push", &[closure, value]);
        }
        Ok((closure, count))
    }

    /// `together`, `together for` (`spec/19`) and `first of` (`spec/27 §6`), as the value of
    /// the expression whose scope is `scope`: each task is a closure of the bindings before the
    /// block, which the runtime runs on a coroutine of its own (`tasks`). The block fails with
    /// the failure of the lowest-indexed task that failed, which for `first of` is only when
    /// every branch failed, and stops when its task must.
    fn together(
        &mut self,
        expr: &'p Expr,
        env: &Env,
        scope: &Scope,
        catch: &Catch,
    ) -> Result<Option<Value>, CodegenError> {
        // The runtime starts its threads before `main` (#1145).
        self.emitter.threads = true;
        let value = match expr {
            Expr::Together { tasks, span } => {
                let site = self.site(*span);
                let closures = self.call("una_rt_list", &[])?;
                for task in tasks {
                    let closure = self.closure(&[], task, env)?;
                    self.runtime("una_rt_push", &[closures, closure]);
                    self.release(closure, Kind::Fn);
                }
                let value = self.call("una_rt_together", &[closures, site])?;
                self.release(closures, Kind::List);
                value
            }
            // `first of` (`spec/27 §6`): each branch a closure, as a `together` task; the
            // runtime races them and gives the winner's value, or the lowest-indexed failure
            // when every branch fails.
            Expr::FirstOf { tasks, span } => {
                let site = self.site(*span);
                let closures = self.call("una_rt_list", &[])?;
                for task in tasks {
                    let closure = self.closure(&[], task, env)?;
                    self.runtime("una_rt_push", &[closures, closure]);
                    self.release(closure, Kind::Fn);
                }
                let value = self.call("una_rt_first_of", &[closures, site])?;
                self.release(closures, Kind::List);
                value
            }
            Expr::TogetherFor {
                source,
                var,
                task,
                span,
            } => {
                let site = self.site(*span);
                let (items, owned) = match source.as_ref() {
                    LoopSource::List(list) => (self.atom(list, env)?.0, false),
                    LoopSource::Range {
                        low,
                        high,
                        inclusive,
                        ..
                    } => {
                        let (low, _) = self.atom(low, env)?;
                        let (high, _) = self.atom(high, env)?;
                        let inclusive =
                            self.builder.ins().iconst(types::I64, i64::from(*inclusive));
                        (
                            self.call("una_rt_range", &[site, low, high, inclusive])?,
                            true,
                        )
                    }
                    LoopSource::Until { .. } | LoopSource::Ever { .. } => {
                        return Err(internal(
                            "a `together for` with an `until` or `ever` source",
                        ));
                    }
                };
                let closure = self.closure(std::slice::from_ref(var.as_ref()), task, env)?;
                let value = self.call("una_rt_together_for", &[closure, items, site])?;
                self.release(closure, Kind::Fn);
                if owned {
                    self.release(items, Kind::List);
                }
                value
            }
            _ => return Err(internal("a `together` that is not one")),
        };
        // A block stopped, status 2, goes on as [`STOPPED`] by the failure path.
        let status = self.call("una_rt_together_status", &[])?;
        let stopped = self.builder.ins().icmp_imm_s(IntCC::Equal, status, 2);
        let word = self.builder.ins().iconst(types::I64, STOPPED);
        let value = self.builder.ins().select(stopped, word, value);
        self.check(value, status, scope, catch);
        for &(owned, kind) in scope.iter().rev() {
            self.release(owned, kind);
        }
        Ok(Some(value))
    }

    /// `tasks(b) body` (`spec/27 §7`, ADR 0117 rule 7): the runtime opens the statement
    /// (`una_rt_tasks_begin`), `body` runs in this task, and every way out of it ends the
    /// statement (`una_rt_tasks_end`), which waits for the tasks it started: its value, after
    /// they ran to their end; its failure or stop, after they were cancelled; and an exit of an
    /// enclosing loop, which crosses the statement, after they ran to their end.
    ///
    /// `body` is lowered with a scope of its own, as `within` lowers its expression, so it
    /// borrows the words of `scope`, which are released here on every path.
    #[allow(clippy::too_many_arguments)]
    fn tasks(
        &mut self,
        bound: Option<&(Atom, Span)>,
        body: &'p Expr,
        span: Span,
        env: &mut Env,
        scope: &mut Scope,
        catch: &Catch,
    ) -> Result<Option<Value>, CodegenError> {
        // The runtime starts its threads before `main` (#1145).
        self.emitter.threads = true;
        let site = self.site(span);
        let (word, bounded) = match bound {
            Some((atom, _)) => {
                let (word, _) = self.atom(atom, env)?;
                (word, self.builder.ins().iconst(types::I64, 1))
            }
            None => {
                let zero = self.builder.ins().iconst(types::I64, 0);
                (zero, zero)
            }
        };
        let begun = self.call("una_rt_tasks_begin", &[word, bounded, site])?;
        // A task that stops before the block runs goes on as [`STOPPED`].
        let stop = self.builder.ins().iconst(types::I64, STOPPED);
        self.check(stop, begun, scope, catch);
        let succeeded = self.join();
        let failed = self.join();
        let done = self.join();
        // The exits of an enclosing loop leave the block through blocks of their own, which
        // end the statement first.
        let crossing = catch.exit.as_ref().map(|outer| {
            let next = self.builder.create_block();
            let stop = self.builder.create_block();
            for _ in &outer.kinds {
                self.builder.append_block_param(next, types::I64);
                self.builder.append_block_param(stop, types::I64);
            }
            (outer.clone(), next, stop)
        });
        let inner = Catch {
            block: failed,
            outer: Scope::new(),
            base: 0,
            exit: crossing.as_ref().map(|(outer, next, stop)| Exit {
                next: *next,
                stop: *stop,
                kinds: outer.kinds.clone(),
                outer: Scope::new(),
                base: 0,
            }),
        };
        let reached = self.branch(body, env, &Scope::new(), succeeded, &inner)?;

        // The block ended: its tasks run to their end.
        self.builder.switch_to_block(succeeded);
        if reached {
            let value = self.param(succeeded)?;
            let keep = self.builder.ins().iconst(types::I64, 0);
            self.runtime("una_rt_tasks_end", &[keep]);
            for &(owned, kind) in scope.iter().rev() {
                self.release(owned, kind);
            }
            self.builder.ins().jump(done, &[value.into()]);
        } else {
            self.builder.ins().trap(TRAP);
        }

        // The block failed or stops: its tasks are cancelled, then the failure or the stop goes
        // on.
        self.builder.switch_to_block(failed);
        let failure = self.param(failed)?;
        let cancel = self.builder.ins().iconst(types::I64, 1);
        self.runtime("una_rt_tasks_end", &[cancel]);
        self.fail(failure, scope, catch);

        // An exit of an enclosing loop: the statement ends, then the exit goes on with the
        // loop's state, releasing what the loop's own exit releases.
        if let Some((outer, next, stop)) = crossing {
            for (block, target) in [(next, outer.next), (stop, outer.stop)] {
                self.builder.switch_to_block(block);
                let state: Vec<Value> = self.builder.block_params(block).to_vec();
                let keep = self.builder.ins().iconst(types::I64, 0);
                self.runtime("una_rt_tasks_end", &[keep]);
                for &(owned, kind) in scope.iter().skip(outer.base).rev() {
                    self.release(owned, kind);
                }
                for &(owned, kind) in outer.outer.iter().rev() {
                    self.release(owned, kind);
                }
                let args: Vec<cranelift_codegen::ir::BlockArg> =
                    state.iter().map(|&value| value.into()).collect();
                self.builder.ins().jump(target, &args);
            }
        }

        self.builder.switch_to_block(done);
        if reached {
            Ok(Some(self.param(done)?))
        } else {
            self.builder.ins().trap(TRAP);
            Ok(None)
        }
    }

    /// `start task` (`spec/27 §7`, ADR 0117 rule 7): `task` as a closure over the bindings it
    /// reads, as a `together` task is, which the runtime starts as a task of the innermost
    /// `tasks` statement, then the code goes on. A start that stops the task, by an abort or
    /// a cancellation, goes on as [`STOPPED`]. Its value is `unit`.
    fn start(
        &mut self,
        task: &'p Expr,
        moves: bool,
        span: Span,
        env: &Env,
        scope: &Scope,
        catch: &Catch,
    ) -> Result<Option<Value>, CodegenError> {
        // The runtime starts its threads before `main` (#1145); a `start` stands only inside
        // a `tasks` statement, which sets it too.
        self.emitter.threads = true;
        let site = self.site(span);
        let (closure, count) = self.counted_closure(&[], task, env)?;
        // The closure's values come first, in the order of their names, then the instances of
        // the capabilities in scope, which are no captured values: the runtime walks the values
        // only, for the reply rights to move (ADR 0123 rule 4.2, `spec/31 §1` rule 3).
        let captures = if moves {
            i64::try_from(count).map_err(crate::internal)?
        } else {
            0
        };
        let captures = self.builder.ins().iconst(types::I64, captures);
        let status = self.call("una_rt_start_task", &[closure, site, captures])?;
        self.release(closure, Kind::Fn);
        let stop = self.builder.ins().iconst(types::I64, STOPPED);
        self.check(stop, status, scope, catch);
        for &(owned, kind) in scope.iter().rev() {
            self.release(owned, kind);
        }
        let (unit, _) = self.atom(&Atom::Unit, env)?;
        Ok(Some(unit))
    }

    /// `attempt body`: `Ok` of the body's value, or `Err` of its failure (`spec/11 §4`), as the
    /// value of the expression whose scope is `scope`.
    fn attempt(
        &mut self,
        body: &'p Expr,
        env: &Env,
        scope: &Scope,
        catch: &Catch,
    ) -> Result<Value, CodegenError> {
        let succeeded = self.join();
        let failed = self.join();
        let done = self.join();
        // Core puts no `skip` or `stop` in the body of `attempt`, a handled expression.
        let inner = Catch {
            block: failed,
            outer: Scope::new(),
            base: scope.len(),
            exit: None,
        };
        // The body's path releases the whole scope when it ends; a failure releases what the
        // body made, and the scope is released here.
        let reached = self.branch(body, env, scope, succeeded, &inner)?;
        self.builder.switch_to_block(succeeded);
        if reached {
            let value = self.param(succeeded)?;
            let ok = self.wrap(0, value)?;
            self.builder.ins().jump(done, &[ok.into()]);
        } else {
            self.builder.ins().trap(TRAP);
        }
        self.builder.switch_to_block(failed);
        let failure = self.param(failed)?;
        // A stop is no failure to hold: it goes on, with the scope still owned.
        self.pass_stop(failure, scope, catch);
        let err = self.wrap(1, failure)?;
        for &(owned, kind) in scope.iter().rev() {
            self.release(owned, kind);
        }
        self.builder.ins().jump(done, &[err.into()]);
        self.builder.switch_to_block(done);
        self.param(done)
    }

    /// `e within d` (`spec/27 §5`, ADR 0117 rule 5): `body` bounded by the deadline
    /// `deadline`. The runtime arms it (`una_rt_within_enter`), and every way out of `body`
    /// ends it (`una_rt_within_end`), which says whether `body`'s outcome stands, the deadline
    /// passed (the time-out `timeout`), or the task stops on.
    ///
    /// `body` is lowered with a scope of its own, so it borrows the words of `scope` and never
    /// releases them: a value that returned too late becomes the time-out with `scope` still
    /// owned, which is released here on every path.
    ///
    /// A passed deadline cancels `body` at a check point. The cancellation leaves each frame of
    /// `body` by the path a failure takes, as [`STOPPED`], so every frame releases what it
    /// owns, and reaches `failed` here, where the `within` turns it into its time-out, or passes
    /// it on.
    #[allow(clippy::too_many_arguments)]
    fn within(
        &mut self,
        deadline: &Atom,
        body: &'p Expr,
        timeout: &Ctor,
        wrap: Option<&Ctor>,
        env: &mut Env,
        scope: &mut Scope,
        catch: &Catch,
    ) -> Result<Option<Value>, CodegenError> {
        // The runtime starts its threads before `main` (#1145): a `within` counts as a task
        // construct for P8, so its cost never depends on whether earlier code started them.
        self.emitter.threads = true;
        let (ms, _) = self.atom(deadline, env)?;
        let id = self.call("una_rt_within_enter", &[ms])?;
        let succeeded = self.join();
        let failed = self.join();
        let done = self.join();
        // A failure of `body` in time, or its stop, goes to `failed`, which releases what
        // `body` made; Core puts no `skip` or `stop` in a handled expression.
        let inner = Catch {
            block: failed,
            outer: Scope::new(),
            base: 0,
            exit: None,
        };
        let reached = self.branch(body, env, &Scope::new(), succeeded, &inner)?;
        let timeout_index = i64::try_from(timeout.index).map_err(crate::internal)?;
        // The time-out: a variant without fields, an immediate word.
        let timed_out = |lowerer: &mut Self| {
            lowerer
                .builder
                .ins()
                .iconst(types::I64, (timeout_index << 1) | 1)
        };

        // `body` returned, in time or not.
        self.builder.switch_to_block(succeeded);
        if reached {
            let value = self.param(succeeded)?;
            let zero = self.builder.ins().iconst(types::I64, 0);
            let outcome = self.call("una_rt_within_end", &[id, zero])?;
            let late = self.builder.create_block();
            let stands = self.builder.create_block();
            self.builder.ins().brif(outcome, late, &[], stands, &[]);
            self.builder.switch_to_block(stands);
            for &(owned, kind) in scope.iter().rev() {
                self.release(owned, kind);
            }
            self.builder.ins().jump(done, &[value.into()]);
            self.builder.switch_to_block(late);
            self.release(value, Kind::Any);
            self.end_within(outcome, timed_out, scope, catch);
        } else {
            self.builder.ins().trap(TRAP);
        }

        // `body` failed with `x`, or stopped. In time, `wrap(x)` or `x`; late, the time-out; a
        // stop, this deadline's time-out, or a stop that goes on.
        self.builder.switch_to_block(failed);
        let failure = self.param(failed)?;
        let stopped = self.stopped(failure);
        let outcome = self.call("una_rt_within_end", &[id, stopped])?;
        let ended = self.builder.create_block();
        let stands = self.builder.create_block();
        self.builder.ins().brif(outcome, ended, &[], stands, &[]);
        self.builder.switch_to_block(stands);
        let passed = match wrap {
            Some(wrap) => {
                let index = i64::try_from(wrap.index).map_err(crate::internal)?;
                self.wrap(index, failure)?
            }
            None => failure,
        };
        self.fail(passed, scope, catch);
        self.builder.switch_to_block(ended);
        self.release(failure, Kind::Any);
        self.end_within(outcome, timed_out, scope, catch);

        self.builder.switch_to_block(done);
        if reached {
            Ok(Some(self.param(done)?))
        } else {
            self.builder.ins().trap(TRAP);
            Ok(None)
        }
    }

    /// The end of a `within` whose `una_rt_within_end` gave `outcome`, 1 or 2, with `scope`
    /// still owned: 1 fails with the time-out that `timed_out` makes, 2 goes on with the stop.
    fn end_within(
        &mut self,
        outcome: Value,
        timed_out: impl Fn(&mut Self) -> Value,
        scope: &Scope,
        catch: &Catch,
    ) {
        let stopping = self.builder.ins().icmp_imm_s(IntCC::Equal, outcome, 2);
        let stopping = self.flag(stopping);
        self.stop_or_fail(stopping, scope, catch);
        let failure = timed_out(self);
        self.fail(failure, scope, catch);
    }

    /// The variant at position `tag` of `Result` or `Option` holding `value`, which it takes
    /// from the caller.
    fn wrap(&mut self, tag: i64, value: Value) -> Result<Value, CodegenError> {
        let tag = self.builder.ins().iconst(types::I64, tag);
        let wrapped = self.call("una_rt_data", &[tag])?;
        self.runtime("una_rt_push", &[wrapped, value]);
        self.release(value, Kind::Any);
        Ok(wrapped)
    }

    /// Fails with `failure`, which the code owns: releases what the code owns since `catch` was
    /// installed, and goes to it.
    fn fail(&mut self, failure: Value, scope: &Scope, catch: &Catch) {
        for &(owned, kind) in scope.iter().skip(catch.base).rev() {
            self.release(owned, kind);
        }
        for &(owned, kind) in catch.outer.iter().rev() {
            self.release(owned, kind);
        }
        self.builder.ins().jump(catch.block, &[failure.into()]);
    }

    /// Continues when `failed` is 0, and fails with `value` otherwise.
    fn check(&mut self, value: Value, failed: Value, scope: &Scope, catch: &Catch) {
        let failing = self.builder.create_block();
        let next = self.builder.create_block();
        self.builder.ins().brif(failed, failing, &[], next, &[]);
        self.builder.switch_to_block(failing);
        self.fail(value, scope, catch);
        self.builder.switch_to_block(next);
    }

    /// The owned value `ctor(args)`.
    fn construct(&mut self, ctor: &Ctor, args: &[Atom], env: &Env) -> Result<Value, CodegenError> {
        let index = i64::try_from(ctor.index).map_err(crate::internal)?;
        // The representation invariant that keeps a failure from equalling [`STOPPED`].
        debug_assert!(index >= 0 && (index << 1) | 1 != STOPPED);
        if args.is_empty() {
            return Ok(self.builder.ins().iconst(types::I64, (index << 1) | 1));
        }
        let tag = self.builder.ins().iconst(types::I64, index);
        let object = self.call("una_rt_data", &[tag])?;
        for arg in args {
            let (value, _) = self.atom(arg, env)?;
            self.runtime("una_rt_push", &[object, value]);
        }
        Ok(object)
    }

    /// The references a call of `function` gives its owned parameters (ADR 0072 D2): an
    /// argument at its last use moves out of `scope`, unless its word is also another argument,
    /// which the callee borrows; any other argument is retained. The callee releases them.
    /// Returns whether the call takes the function's borrowed entry instead (#284): when no
    /// argument of an owned parameter would move, each would be retained only for the callee
    /// to release it, so the call gives nothing, and the entry that borrows every parameter
    /// copies a value only where it consumes it, as the owned entry would copy the retained one.
    #[allow(clippy::too_many_arguments)]
    fn give(
        &mut self,
        function: &Path,
        args: &[Atom],
        values: &[Value],
        env: &Env,
        scope: &mut Scope,
        catch: &Catch,
        rest: &Expr,
    ) -> Result<bool, CodegenError> {
        let owned = self.emitter.owned(function);
        if owned.contains(&true)
            && !(0..args.len()).any(|index| {
                owned.get(index).copied().unwrap_or(false)
                    && self.moves(index, args, values, env, scope, catch, rest)
            })
        {
            return Ok(true);
        }
        self.hand_over(&owned, args, values, env, scope, catch, rest)?;
        Ok(false)
    }

    /// Whether the argument at `index` moves into the callee: it is at its last use in `scope`,
    /// and its word is no other argument, which the callee borrows.
    #[allow(clippy::too_many_arguments)]
    fn moves(
        &self,
        index: usize,
        args: &[Atom],
        values: &[Value],
        env: &Env,
        scope: &Scope,
        catch: &Catch,
        rest: &Expr,
    ) -> bool {
        let (Some(arg), Some(value)) = (args.get(index), values.get(index)) else {
            return false;
        };
        let shared = values
            .iter()
            .enumerate()
            .any(|(other, word)| other != index && word == value);
        !shared && self.moving(arg, env, scope, catch, rest).is_some()
    }

    /// [`Lowerer::give`] for the parameters `owned` says are owned, of a function or of a
    /// lambda.
    #[allow(clippy::too_many_arguments)]
    fn hand_over(
        &mut self,
        owned: &[bool],
        args: &[Atom],
        values: &[Value],
        env: &Env,
        scope: &mut Scope,
        catch: &Catch,
        rest: &Expr,
    ) -> Result<(), CodegenError> {
        for (index, arg) in args.iter().enumerate() {
            if !owned.get(index).copied().unwrap_or(false) {
                continue;
            }
            let (value, kind) = self.atom(arg, env)?;
            let shared = values
                .iter()
                .enumerate()
                .any(|(other, word)| other != index && *word == value);
            match self.moving(arg, env, scope, catch, rest) {
                Some(position) if !shared => {
                    scope.remove(position);
                }
                _ => self.retain(value, kind),
            }
        }
        Ok(())
    }

    /// The owned result of a call of the function or predicate `function` with `args` at
    /// `span`, through its borrowed entry when `borrowed` (#284); its failure goes to `catch`.
    #[allow(clippy::too_many_arguments)]
    fn call_function(
        &mut self,
        function: &Path,
        borrowed: bool,
        type_args: &[Type],
        args: &[Value],
        env: &Env,
        scope: &Scope,
        catch: &Catch,
    ) -> Result<Value, CodegenError> {
        let callee = self.emitter.function_entry(function, borrowed)?;
        let mut args = args.to_vec();
        // After the arguments, the type descriptor of each constrained type parameter: a
        // constant for a known type, the caller's own for its type parameter (`spec/20 §7`).
        for (index, &constraint) in callee.constraints.iter().enumerate() {
            if constraint == Constraint::None {
                continue;
            }
            let ty = type_args.get(index).ok_or_else(|| {
                internal(format!(
                    "a call of `{}` lacks a type argument",
                    function.qualified()
                ))
            })?;
            args.push(self.descriptor(ty, constraint, env)?);
        }
        for name in &callee.uses {
            let (instance, _) = env
                .get(&cap_key(name))
                .ok_or_else(|| internal(format!("no capability `@{name}` to pass")))?;
            args.push(*instance);
        }
        let id = callee.id;
        let callee = self
            .emitter
            .module
            .declare_func_in_func(id, self.builder.func);
        let call = self.builder.ins().call(callee, &args);
        let results = self.builder.inst_results(call).to_vec();
        self.runtime("una_rt_leave", &[]);
        let (Some(&value), Some(&failed)) = (results.first(), results.get(1)) else {
            return Err(internal("a call lost its result"));
        };
        // A callee that stopped returns [`STOPPED`] with status 2, a failure to pass on.
        // Every callee returns whether it failed, 0 when it cannot: a function with `fails of
        // p` fails when the function value given for `p` does, which its declaration does not
        // say, so every call checks (#267).
        self.check(value, failed, scope, catch);
        Ok(value)
    }

    /// `c.o(a, ...)` (E-OpHost, E-OpHandler, `spec/11 §4`), whose result has type `ty`: records
    /// the operation's event, performs it on the instance bound to `c`, a host instance through
    /// the runtime or a handler's through its code, and ends the event with its outcome; a
    /// failure then goes to `catch`.
    fn op(
        &mut self,
        rhs: &'p Rhs,
        ty: &Type,
        env: &Env,
        scope: &Scope,
        catch: &Catch,
    ) -> Result<Value, CodegenError> {
        let Rhs::Op {
            cap,
            capability,
            capability_path,
            op,
            args,
            labels,
            outcome,
            span,
            ..
        } = rhs
        else {
            return Err(internal("an operation that is not one"));
        };
        let ops = self
            .emitter
            .capability(capability_path)
            .ok_or_else(|| internal(format!("no capability `{}`", capability_path.qualified())))?;
        let (index, declared) = ops
            .iter()
            .enumerate()
            .find(|(_, o)| o.name == *op)
            .ok_or_else(|| internal(format!("`{capability}` has no operation `{op}`")))?;
        let (instance, _) = *env
            .get(&cap_key(cap))
            .ok_or_else(|| internal(format!("no capability `@{cap}`")))?;
        // What the event records: the arguments, redacted by their static labels, the result,
        // redacted by its type's label, and the failure (`spec/11 §5`).
        let mut values = Vec::new();
        let mut shown = Vec::new();
        for (position, arg) in args.iter().enumerate() {
            values.push(self.atom(arg, env)?.0);
            let arg_type = self.atom_type(arg, env);
            let label = labels.get(position).cloned().unwrap_or_else(Label::public);
            shown.push(serde_json::json!({
                "type": self.emitter.types.of(&arg_type),
                "redacted": redaction(&label),
            }));
        }
        let result = serde_json::json!({
            "type": self.emitter.types.of(ty),
            "redacted": redaction(&ty.deep_label()),
        });
        // S-45 (#1552, `spec/11 §5`): the level an outcome labeled above `public` is written at,
        // `redacted`, joined with the binder's label as the interpreter joins it.
        let outcome = redaction(&outcome.join(&ty.deep_label()));
        let error = match (&declared.fails, &declared.error) {
            (None, _) => None,
            (Some(_), Some(kind)) => Some(self.emitter.types.of(&base(Base::Named {
                module: kind.module.clone(),
                name: kind.name.clone(),
                args: Vec::new(),
            }))),
            (Some(_), None) => {
                return Err(internal(format!(
                    "the error kind of `{capability}.{op}` is not resolved"
                )));
            }
        };
        let params: Vec<String> = declared
            .params
            .iter()
            .map(|(name, _)| source_name(name))
            .collect();
        let site = i64::try_from(self.emitter.ops.len()).map_err(crate::internal)?;
        // Where an abort at the operation is reported, a value of its event whose text does not
        // fit the memory budget (`spec/11 §7`, #1223), as the interpreter reports it.
        let position = self.emitter.sites.len();
        self.emitter.sites.push(*span);
        self.emitter.ops.push(serde_json::json!({
            "capability": capability,
            "op": op,
            "params": params,
            "args": shown,
            "result": result,
            "outcome": outcome,
            "error": error,
            "site": position,
        }));
        let site = self.builder.ins().iconst(types::I64, site);
        // A cancelled task stops before its next operation (`spec/19 §6` rule 4).
        let stopping = self.call("una_rt_poll", &[])?;
        self.stop_or_fail(stopping, scope, catch);
        // From here the operation is in progress: it completes, and keeps its own outcome,
        // whatever cancellation comes meanwhile (`spec/19 §6` rule 4, `§7` rule 3).
        self.runtime("una_rt_operation_begin", &[]);
        let slot = self.call("una_rt_event", &[site, instance])?;
        for (position, &value) in values.iter().enumerate() {
            let position = i64::try_from(position).map_err(crate::internal)?;
            let position = self.builder.ins().iconst(types::I64, position);
            self.runtime("una_rt_event_arg", &[site, slot, position, value]);
        }
        // An argument whose text does not fit the memory budget stopped the task (`spec/11 §7`,
        // #1223): the operation does not start, as in the interpreter; in `main`'s thread the
        // run has ended.
        if !values.is_empty() {
            let stopped = self.call("una_rt_stopped", &[])?;
            self.stop_if(stopped);
        }
        let index = i64::try_from(index).map_err(crate::internal)?;
        let index = self.builder.ins().iconst(types::I64, index);
        let code = self.call("una_rt_op_code", &[instance, index])?;
        let host = self.builder.create_block();
        let handled = self.builder.create_block();
        let done = self.builder.create_block();
        self.builder.append_block_param(done, types::I64);
        self.builder.append_block_param(done, types::I64);
        self.builder.ins().brif(code, handled, &[], host, &[]);

        // A host instance has no code: the runtime performs the operation (`spec/14 §3`); so
        // does the instance of `limiter` of `std.limit` (ADR 0117 rule 9, `spec/27 §10`).
        self.builder.switch_to_block(host);
        let limit = capability_path.module == una_core::LIMIT_MODULE;
        if capability_path.module == HOST_MODULE || limit {
            let (function, fails) = match (capability_path.name.as_str(), op.as_str()) {
                ("Limit", "acquire") if limit => ("una_rt_limit_acquire", true),
                _ if limit => return Err(unsupported(format!("`{capability}.{op}`"))),
                ("Console", "print") => ("una_rt_console_print", false),
                ("Clock", "now") => ("una_rt_clock_now", false),
                ("Env", "get") => ("una_rt_env_get", false),
                ("Files", "read") => ("una_rt_files_read", true),
                ("Files", "write") => ("una_rt_files_write", true),
                ("Keys", "hmac") => ("una_rt_keys_hmac", true),
                ("Http", "get") => ("una_rt_http_get", true),
                ("Http", "post") => ("una_rt_http_post", true),
                ("Http", "put") => ("una_rt_http_put", true),
                ("Http", "delete") => ("una_rt_http_delete", true),
                // ADR 0123 rules 1 to 4: over the run's serve engine (`una_rt::serve`).
                ("HttpServer", "next") => ("una_rt_serve_next", true),
                ("HttpServer", "reply") => ("una_rt_serve_reply", true),
                ("HttpServer", "close") => ("una_rt_serve_close", false),
                _ => return Err(unsupported(format!("`{capability}.{op}`"))),
            };
            if capability_path.name == "Http" && !limit {
                self.emitter.http = true;
            }
            if capability_path.name == "HttpServer" {
                self.emitter.serve = true;
            }
            let sited = limit
                || matches!(
                    (capability_path.name.as_str(), op.as_str()),
                    ("Http" | "HttpServer", _) | ("Env", "get") | ("Files", "read")
                );
            let value = if sited {
                // The call's site, where a header value with a control character aborts with
                // U6004, and host text that is not valid UTF-8 with U6005 (S-16, ADR
                // 0086); a task stopped by it stops here, as one whose wait at a `limiter`
                // was interrupted does.
                let site = self.site(*span);
                let mut operands = vec![site];
                // `acquire` takes its instance, whose bucket the runtime holds.
                if limit {
                    operands.push(instance);
                }
                operands.extend_from_slice(&values);
                let value = self.call(function, &operands)?;
                let stopped = self.call("una_rt_stopped", &[])?;
                self.stop_if(stopped);
                value
            } else {
                self.call(function, &values)?
            };
            let failed = if fails {
                self.call("una_rt_host_failed", &[])?
            } else {
                self.builder.ins().iconst(types::I64, 0)
            };
            self.runtime("una_rt_operation_end", &[]);
            self.builder
                .ins()
                .jump(done, &[value.into(), failed.into()]);
        } else {
            // Only `una run` makes host instances, of host capabilities.
            self.builder.ins().trap(TRAP);
        }

        // A handler's operation counts toward the maximum depth, as a call does.
        self.builder.switch_to_block(handled);
        let call_site = self.site(*span);
        let mut signature = self.emitter.module.make_signature();
        for _ in 0..=values.len() {
            signature.params.push(AbiParam::new(types::I64));
        }
        signature.returns.push(AbiParam::new(types::I64));
        signature.returns.push(AbiParam::new(types::I64));
        let signature = self.builder.import_signature(signature);
        let mut operands = vec![instance];
        operands.extend_from_slice(&values);
        // A task that stops at the operation's call, by U6003 alone since the operation shields
        // cancellations, or inside it, ends the operation with [`STOPPED`] and status 2, which
        // leaves its event as it was, `aborted`, and goes on as a failure.
        let stopping = self.call("una_rt_enter", &[call_site])?;
        let refused = self.builder.create_block();
        let calling = self.builder.create_block();
        self.builder
            .ins()
            .brif(stopping, refused, &[], calling, &[]);
        self.builder.switch_to_block(refused);
        self.runtime("una_rt_operation_end", &[]);
        let word = self.builder.ins().iconst(types::I64, STOPPED);
        let two = self.builder.ins().iconst(types::I64, 2);
        self.builder.ins().jump(done, &[word.into(), two.into()]);
        self.builder.switch_to_block(calling);
        let call = self.builder.ins().call_indirect(signature, code, &operands);
        let results = self.builder.inst_results(call).to_vec();
        self.runtime("una_rt_leave", &[]);
        self.runtime("una_rt_operation_end", &[]);
        let (Some(&value), Some(&failed)) = (results.first(), results.get(1)) else {
            return Err(internal("an operation lost its result"));
        };
        self.builder
            .ins()
            .jump(done, &[value.into(), failed.into()]);

        self.builder.switch_to_block(done);
        let results = self.builder.block_params(done).to_vec();
        let (Some(&value), Some(&failed)) = (results.first(), results.get(1)) else {
            return Err(internal("an operation lost its result"));
        };
        self.runtime("una_rt_event_end", &[site, slot, value, failed]);
        // An outcome whose text does not fit the memory budget stopped the task (`spec/11 §7`,
        // #1223): it stops here, before it uses the value, as in the interpreter. An operation
        // that ended with status 2 stopped at or in its call and goes on as a failure.
        let stopped = self.call("una_rt_stopped", &[])?;
        let ended = self.builder.ins().icmp_imm_s(IntCC::NotEqual, failed, 2);
        let ended = self.builder.ins().uextend(types::I64, ended);
        let stopping = self.builder.ins().band(stopped, ended);
        self.stop_if(stopping);
        // An operation that cannot fail returns 0 as its failure flag, so every operation
        // checks it, as every call does (#267).
        self.check(value, failed, scope, catch);
        Ok(value)
    }

    /// A new instance of `handler` (E-Handler, `spec/11 §4`): the code of each operation of its
    /// capability, in the capability's order, then the values `args` and the instances of its
    /// own capabilities, from those in scope.
    fn install(&mut self, handler: &Path, args: &[Atom], env: &Env) -> Result<Value, CodegenError> {
        // `limiter` of `std.limit` (ADR 0117 rule 9, `spec/27 §10`): the runtime makes the
        // instance, one bucket over the run's clock, from `at_most`, `per` and `waiting`.
        if handler.module == una_core::LIMIT_MODULE && handler.name == una_core::LIMITER {
            let mut operands = Vec::new();
            for arg in args {
                operands.push(self.atom(arg, env)?.0);
            }
            return self.call("una_rt_limiter_instance", &operands);
        }
        let info = self
            .emitter
            .handler(handler)
            .ok_or_else(|| internal(format!("no handler `{}`", handler.qualified())))?;
        let ops = self
            .emitter
            .capability(info.capability)
            .ok_or_else(|| internal(format!("no capability `{}`", info.capability.qualified())))?;
        let name = self.literal("una_rt_text_literal", &handler.name)?;
        let instance = self.call("una_rt_handler_instance", &[name])?;
        for op in ops {
            let id = self.emitter.handler_op(handler, op)?;
            let code = self
                .emitter
                .module
                .declare_func_in_func(id, self.builder.func);
            let address = self.builder.ins().func_addr(types::I64, code);
            self.runtime("una_rt_push_op", &[instance, address]);
        }
        for arg in args {
            let (value, _) = self.atom(arg, env)?;
            self.runtime("una_rt_push", &[instance, value]);
        }
        for need in info.uses {
            let (value, _) = *env
                .get(&cap_key(&need.cap))
                .ok_or_else(|| internal(format!("no capability `@{}`", need.cap)))?;
            self.runtime("una_rt_push", &[instance, value]);
        }
        Ok(instance)
    }

    /// The owned word of a right-hand side bound to a binder of type `ty`; a failure goes to
    /// `catch`.
    fn rhs(
        &mut self,
        rhs: &'p Rhs,
        ty: &Type,
        env: &Env,
        scope: &mut Scope,
        catch: &Catch,
        rest: &'p Expr,
    ) -> Result<Value, CodegenError> {
        match rhs {
            Rhs::Tuple(items) => {
                let zero = self.builder.ins().iconst(types::I64, 0);
                let tuple = self.call("una_rt_data", &[zero])?;
                for item in items {
                    let (value, _) = self.atom(item, env)?;
                    self.runtime("una_rt_push", &[tuple, value]);
                }
                Ok(tuple)
            }
            Rhs::Atom(atom) => {
                let (value, kind) = self.atom(atom, env)?;
                // `let b = a` at the last use of `a` moves it to `b` (#30).
                if let Some(position) = self.moving(atom, env, scope, catch, rest) {
                    scope.remove(position);
                    return Ok(value);
                }
                self.retain(value, kind);
                Ok(value)
            }
            // A map's methods (`spec/21 §2`): `with` and `without` at the map's last use take it
            // over, and update it in place when nothing else holds it (`§5` rule 2).
            Rhs::Prim {
                prim: Prim::Map(op),
                args,
                ..
            } => {
                let mut values = Vec::new();
                for arg in args {
                    values.push(self.atom(arg, env)?.0);
                }
                let moved = match (op, args.first()) {
                    (MapOp::With | MapOp::Without, Some(map)) => {
                        self.moving(map, env, scope, catch, rest)
                    }
                    _ => None,
                };
                if let Some(position) = moved {
                    scope.remove(position);
                }
                let runtime = match (op, moved.is_some()) {
                    (MapOp::Has, _) => "una_rt_map_has",
                    (MapOp::Get, _) => "una_rt_map_get",
                    (MapOp::With, false) => "una_rt_map_with",
                    (MapOp::With, true) => "una_rt_map_with_owned",
                    (MapOp::Without, false) => "una_rt_map_without",
                    (MapOp::Without, true) => "una_rt_map_without_owned",
                    (MapOp::Keys | MapOp::Values | MapOp::Entries, _) => {
                        let part = match op {
                            MapOp::Keys => 0,
                            MapOp::Values => 1,
                            _ => 2,
                        };
                        let part = self.builder.ins().iconst(types::I64, part);
                        values.push(part);
                        "una_rt_map_list"
                    }
                };
                self.call(runtime, &values)
            }
            // The methods of the collections of `std.collections` (`spec/28`): an operation that
            // gives a new collection takes its receiver over at its last use, and updates it in
            // place when nothing else holds it (ADR 0119 rule 1), unless the receiver is also
            // another argument, which the update would change.
            Rhs::Prim {
                prim:
                    prim @ (Prim::Seq(_)
                    | Prim::Set(_)
                    | Prim::HashMap(_)
                    | Prim::HashSet(_)
                    | Prim::Priority(_)
                    | Prim::BitSet(_)),
                args,
                span,
                ..
            } => {
                let mut values = Vec::new();
                for arg in args {
                    values.push(self.atom(arg, env)?.0);
                }
                let (code, rebuilds) = collection_code(*prim)?;
                let moved = match args.split_first() {
                    Some((receiver, others)) if rebuilds && !others.contains(receiver) => {
                        self.moving(receiver, env, scope, catch, rest)
                    }
                    _ => None,
                };
                if let Some(position) = moved {
                    scope.remove(position);
                }
                let owned = moved.is_some();
                // A hash table hashes its keys by the descriptor of its key type, the first type
                // argument of the receiver's type (`spec/28 §1` rule 2): a word does not say its
                // type.
                let desc = match prim {
                    Prim::HashMap(_) | Prim::HashSet(_) => {
                        let ty = match args.first() {
                            Some(receiver) => self.atom_type(receiver, env),
                            None => unknown(),
                        };
                        let key = match &ty {
                            Type::Base {
                                base:
                                    Base::Named {
                                        args: type_args, ..
                                    },
                                ..
                            } => type_args.first().cloned(),
                            _ => None,
                        }
                        .ok_or_else(|| internal("a hash table without its key type"))?;
                        Some(self.descriptor(&key, Constraint::Eq, env)?)
                    }
                    _ => None,
                };
                let code = self.builder.ins().iconst(types::I64, code);
                let zero = self.builder.ins().iconst(types::I64, 0);
                let word = |i: usize| values.get(i).copied().unwrap_or(zero);
                let (receiver, a, b) = (word(0), word(1), word(2));
                match prim {
                    Prim::Seq(_) => {
                        let function = if owned {
                            "una_rt_seq_op_owned"
                        } else {
                            "una_rt_seq_op"
                        };
                        self.call(function, &[code, receiver, a])
                    }
                    Prim::Set(_) => {
                        let function = if owned {
                            "una_rt_set_op_owned"
                        } else {
                            "una_rt_set_op"
                        };
                        self.call(function, &[code, receiver, a, b])
                    }
                    Prim::Priority(_) => {
                        let function = if owned {
                            "una_rt_priority_op_owned"
                        } else {
                            "una_rt_priority_op"
                        };
                        self.call(function, &[code, receiver, a, b])
                    }
                    // A hash table compares keys within the memory budget, as `==` does, so it
                    // takes the call's site and stops after it on U6003.
                    Prim::HashMap(_) => {
                        let function = if owned {
                            "una_rt_hash_map_op_owned"
                        } else {
                            "una_rt_hash_map_op"
                        };
                        let desc = desc.unwrap_or(zero);
                        self.set(function, &[code, desc, receiver, a, b], *span)
                    }
                    Prim::HashSet(_) => {
                        let function = if owned {
                            "una_rt_hash_set_op_owned"
                        } else {
                            "una_rt_hash_set_op"
                        };
                        let desc = desc.unwrap_or(zero);
                        self.set(function, &[code, desc, receiver, a], *span)
                    }
                    // A bit set's precondition aborts at the call, with U8001 (ADR 0119 rule
                    // 14), so it takes the call's site and stops after it.
                    _ => {
                        let function = if owned {
                            "una_rt_bit_set_op_owned"
                        } else {
                            "una_rt_bit_set_op"
                        };
                        self.set(function, &[code, receiver, a], *span)
                    }
                }
            }
            Rhs::Prim {
                prim, args, span, ..
            } => {
                // On values of a constrained type parameter, comparisons and `to_int()` read the
                // parameter's type descriptor (`spec/20 §7`).
                if let Some(desc) = args
                    .first()
                    .and_then(|first| self.descriptor_of(first, env))
                {
                    let mut operands = vec![desc];
                    for arg in args {
                        operands.push(self.atom(arg, env)?.0);
                    }
                    return self.generic_prim(*prim, &operands, *span);
                }
                // Interpolation of a value other than an integer, a `Bool` or a `Text` writes
                // it by its type's description (`spec/11 §5`, ADR 0068).
                if *prim == Prim::ToText
                    && let Some(first) = args.first()
                {
                    let ty = self.atom_type(first, env);
                    if !matches!(
                        &ty,
                        Type::Base {
                            base: Base::Int | Base::Fixed(_) | Base::Bool | Base::Text,
                            ..
                        }
                    ) {
                        let (value, _) = self.atom(first, env)?;
                        // The site, where a text that does not fit the memory budget aborts
                        // the run (`spec/11 §7`, #1223); a task stopped by it stops here.
                        let site = self.site(*span);
                        // A type that holds type parameters is written through its composite
                        // descriptor (ADR 0068 D6).
                        if !type_params(&ty).is_empty() {
                            let desc = self.descriptor(&ty, Constraint::Eq, env)?;
                            let text = self.call("una_rt_generic_write", &[site, desc, value])?;
                            return self.stopped_after(text);
                        }
                        let index =
                            i64::try_from(self.emitter.types.of(&ty)).map_err(crate::internal)?;
                        let index = self.builder.ins().iconst(types::I64, index);
                        let text = self.call("una_rt_write", &[site, index, value])?;
                        return self.stopped_after(text);
                    }
                }
                // `"{t}..."` at the last use of the text of `t` takes it over, with every owned
                // reference to it, as `t` beside the temporary of its conversion to text, and
                // appends to it in place when nothing else holds it (ADR 0070 X10, #261). Not
                // when a later part is the same text, which the first append would change.
                if *prim == Prim::Concat
                    && let Some((first, parts)) = args.split_first()
                    && !parts.is_empty()
                    && let Some(position) = self.moving(first, env, scope, catch, rest)
                {
                    let (text, _) = self.atom(first, env)?;
                    let mut values = Vec::new();
                    for part in parts {
                        values.push(self.atom(part, env)?.0);
                    }
                    if !values.contains(&text) {
                        scope.remove(position);
                        while let Some(other) = scope
                            .iter()
                            .enumerate()
                            .skip(catch.base)
                            .find(|(_, (owned, _))| *owned == text)
                            .map(|(other, _)| other)
                        {
                            scope.remove(other);
                            self.release(text, Kind::Text);
                        }
                        let mut text = text;
                        for part in values {
                            text = self.call("una_rt_text_append_owned", &[text, part])?;
                        }
                        return Ok(text);
                    }
                }
                let mut operands = Vec::new();
                let mut machine = true;
                for arg in args {
                    operands.push(self.atom(arg, env)?);
                    machine &= match arg {
                        Atom::Int(digits) => {
                            digits.parse::<i64>().is_ok_and(|n| SMALL.contains(&n))
                        }
                        Atom::Var(name) => env
                            .get(name.as_str())
                            .is_some_and(|(_, ty)| range::machine_width(ty)),
                        _ => false,
                    };
                }
                let width = Width {
                    operands: machine && !self.exact,
                    result: range::machine_width(ty) && !self.exact,
                };
                self.prim(*prim, &operands, width, *span)
            }
            Rhs::Call {
                function,
                type_args,
                args,
                span,
                ..
            } => {
                let mut values = Vec::new();
                for arg in args {
                    values.push(self.atom(arg, env)?.0);
                }
                // The generic functions of the prelude take the descriptor of their type
                // argument first (`spec/20 §6`, `§7`). In edition 0.1, `min`, `max` and `sum`
                // are on `Int` and `contains` compares structurally, with no type argument.
                if function.module == PRELUDE_MODULE {
                    let generic = match function.name.as_str() {
                        "min" => Some(("una_rt_generic_min", Constraint::Ord, Descriptor::Int)),
                        "max" => Some(("una_rt_generic_max", Constraint::Ord, Descriptor::Int)),
                        "sort" => Some(("una_rt_generic_sort", Constraint::Ord, Descriptor::Int)),
                        "sum" => Some(("una_rt_generic_sum", Constraint::Integer, Descriptor::Int)),
                        "contains" => Some((
                            "una_rt_generic_contains_at",
                            Constraint::Eq,
                            Descriptor::Structural(0),
                        )),
                        _ => None,
                    };
                    if let Some((runtime, constraint, default)) = generic {
                        let desc = match type_args.first() {
                            Some(ty) => self.descriptor(ty, constraint, env)?,
                            None => self.builder.ins().iconst(types::I64, default.word()),
                        };
                        // `contains` compares within the memory budget, at this call's site.
                        if runtime == "una_rt_generic_contains_at" {
                            let mut operands = vec![desc];
                            operands.extend(values);
                            return self.set(runtime, &operands, *span);
                        }
                        let mut operands = vec![desc];
                        operands.extend(values);
                        return self.call(runtime, &operands);
                    }
                }
                // Dense values (`spec/29 §17`, ADR 0120): the runtime's entry points.
                if function.module == PRELUDE_MODULE && is_dense(&function.name) {
                    return self.dense_call(
                        &function.name,
                        args,
                        &values,
                        *span,
                        env,
                        scope,
                        catch,
                        rest,
                    );
                }
                if function.module == PRELUDE_MODULE {
                    // `append` and `set` at the last use of their list take it over, and update
                    // it in place when nothing else holds it (#30).
                    let owned = match function.name.as_str() {
                        "append" => Some("una_rt_append_owned"),
                        "set" => Some("una_rt_set_owned"),
                        _ => None,
                    };
                    if let (Some(owned), Some(list)) = (owned, args.first())
                        && let Some(position) = self.moving(list, env, scope, catch, rest)
                    {
                        scope.remove(position);
                        if function.name == "set" {
                            return self.set(owned, &values, *span);
                        }
                        return self.call(owned, &values);
                    }
                    // `fold` hands its state to the step as owned: an initial value at
                    // its last use moves in, unless it is the list folded over, which the loop
                    // still reads.
                    if function.name == "fold" {
                        let shared = values.get(1).is_some() && values.first() == values.get(1);
                        let moved = match args.get(1) {
                            Some(initial) if !shared => {
                                self.moving(initial, env, scope, catch, rest)
                            }
                            _ => None,
                        };
                        if let Some(position) = moved {
                            scope.remove(position);
                        }
                        let site = self.site(*span);
                        return self.higher_order(
                            "fold",
                            &values,
                            site,
                            moved.is_some(),
                            scope,
                            catch,
                        );
                    }
                    return self.prelude(&function.name, &values, *span, scope, catch);
                }
                // The functions of `std.collections` whose bodies are native (`spec/28`).
                if function.module == una_core::COLLECTIONS_MODULE
                    && let Some(made) =
                        self.collections(&function.name, &values, type_args, *span, env)?
                {
                    return Ok(made);
                }
                // `spec/11 §7`: each call counts toward the maximum depth, which the runtime
                // checks against the call's site before the arguments are given, so a stop
                // there leaves with every word still in scope.
                let site = self.site(*span);
                self.enter(site, scope, catch)?;
                let borrowed = self.give(function, args, &values, env, scope, catch, rest)?;
                self.call_function(function, borrowed, type_args, &values, env, scope, catch)
            }
            Rhs::Apply {
                callee, args, span, ..
            } => {
                let (callee, _) = self.atom(callee, env)?;
                let mut values = Vec::new();
                for arg in args {
                    values.push(self.atom(arg, env)?.0);
                }
                // A lambda owns every parameter: each argument moves at its last use,
                // and is retained otherwise.
                let owned = vec![true; args.len()];
                self.hand_over(&owned, args, &values, env, scope, catch, rest)?;
                let site = self.site(*span);
                let (value, failed) = self.apply(callee, &values, site)?;
                // A function value's type does not say whether it fails when it is the parameter
                // of a `fails of` clause, so every call of one checks (#267).
                self.check(value, failed, scope, catch);
                Ok(value)
            }
            Rhs::Op { .. } => self.op(rhs, ty, env, scope, catch),
            Rhs::Construct { ctor, args, .. } if !args.is_empty() => {
                // A value of the same kind that dies here lends its object, when nothing else
                // holds it: a field edit `r.f = v` rebuilds `r` (#30).
                if let Some((old, position)) = self.reusable(ty, args, env, scope, catch, rest) {
                    scope.remove(position);
                    let index = i64::try_from(ctor.index).map_err(crate::internal)?;
                    let tag = self.builder.ins().iconst(types::I64, index);
                    let object = self.call("una_rt_reuse", &[old, tag])?;
                    for arg in args {
                        let (value, _) = self.atom(arg, env)?;
                        self.runtime("una_rt_push", &[object, value]);
                    }
                    return Ok(object);
                }
                self.construct(ctor, args, env)
            }
            Rhs::Construct { ctor, args, .. } => self.construct(ctor, args, env),
            Rhs::Field { value, index, .. } => {
                let (record, _) = self.atom(value, env)?;
                let field = self.field(record, *index)?;
                self.retain(field, kind(ty)?);
                Ok(field)
            }
            Rhs::List(items) => {
                let list = self.call("una_rt_list", &[])?;
                for item in items {
                    let (value, _) = self.atom(item, env)?;
                    self.runtime("una_rt_push", &[list, value]);
                }
                Ok(list)
            }
            Rhs::Lambda { params, body, .. } => self.closure(params, body, env),
            Rhs::Checked {
                value,
                it,
                predicate,
                ctor,
                args,
                ..
            } => {
                // The value when the predicate holds of it, else the failure `ctor(args)`.
                let (checked, kind) = self.atom(value, env)?;
                let mut inner = env.clone();
                inner.insert(it.clone(), (checked, self.atom_type(value, env)));
                let nested = catch.nested(scope);
                // The verifier reads the predicate as its formula, exact, with no `overflow`
                // obligation, so it is computed exactly (#180).
                let outer = std::mem::replace(&mut self.exact, true);
                let holds = self.expr(predicate, &mut inner, &mut Scope::new(), &nested);
                self.exact = outer;
                let Some(holds) = holds? else {
                    // The predicate never returns, and nothing after it runs.
                    let unreachable = self.builder.create_block();
                    self.builder.switch_to_block(unreachable);
                    return Ok(self.builder.ins().iconst(types::I64, 0));
                };
                let passed = self.builder.create_block();
                let failing = self.builder.create_block();
                self.builder.ins().brif(holds, passed, &[], failing, &[]);
                self.builder.switch_to_block(failing);
                let failure = self.construct(ctor, args, env)?;
                self.fail(failure, scope, catch);
                self.builder.switch_to_block(passed);
                self.retain(checked, kind);
                Ok(checked)
            }
            Rhs::Declassify { via, arg, span, .. } => {
                let (value, _) = self.atom(arg, env)?;
                let site = self.site(*span);
                self.enter(site, scope, catch)?;
                let borrowed = self.give(
                    via,
                    std::slice::from_ref(arg),
                    &[value],
                    env,
                    scope,
                    catch,
                    rest,
                )?;
                self.call_function(via, borrowed, &[], &[value], env, scope, catch)
            }
        }
    }

    /// The borrowed word of an atom, and its kind.
    fn atom(&mut self, atom: &Atom, env: &Env) -> Result<(Value, Kind), CodegenError> {
        Ok(match atom {
            Atom::Var(name) => {
                let (value, ty) = env
                    .get(name.as_str())
                    .ok_or_else(|| internal(format!("`{name}` is not bound")))?;
                (*value, kind(ty)?)
            }
            Atom::Bool(value) => (
                self.builder.ins().iconst(types::I64, i64::from(*value)),
                Kind::Bool,
            ),
            Atom::Unit => (self.builder.ins().iconst(types::I64, 0), Kind::Unit),
            Atom::Int(digits) => match digits.parse::<i64>() {
                Ok(small) if SMALL.contains(&small) => (
                    self.builder.ins().iconst(types::I64, (small << 1) | 1),
                    Kind::Int,
                ),
                _ => (self.literal("una_rt_int_literal", digits)?, Kind::Int),
            },
            Atom::Text(text) => (self.literal("una_rt_text_literal", text)?, Kind::Text),
            // A literal is made once by the runtime and borrowed, as an integer literal is.
            Atom::F64(bits) => {
                let bits = self
                    .builder
                    .ins()
                    .iconst(types::I64, i64::from_ne_bytes(bits.to_ne_bytes()));
                (self.call("una_rt_f64_literal", &[bits])?, Kind::F64)
            }
            // A decimal literal is its coefficient's word (ADR 0120 rule 20).
            Atom::Decimal { coefficient, .. } => match coefficient.parse::<i64>() {
                Ok(small) if SMALL.contains(&small) => (
                    self.builder.ins().iconst(types::I64, (small << 1) | 1),
                    Kind::Int,
                ),
                _ => (self.literal("una_rt_int_literal", coefficient)?, Kind::Int),
            },
            // The tagged word of its totalOrder key, an immediate (ADR 0120 rule 10).
            Atom::F32(bits) => {
                let key = i64::from(una_core::float32::order_key(f32::from_bits(*bits)));
                (
                    self.builder.ins().iconst(types::I64, (key << 1) | 1),
                    Kind::F32,
                )
            }
        })
    }

    /// The type of an atom.
    /// The word of the type descriptor for `ty` under `constraint` (`spec/20 §7`).
    fn descriptor(
        &mut self,
        ty: &Type,
        constraint: Constraint,
        env: &Env,
    ) -> Result<Value, CodegenError> {
        let types = &mut self.emitter.types;
        match constant_descriptor(ty, constraint, &mut |ty| types.of(ty)) {
            Some(descriptor) => Ok(self.builder.ins().iconst(types::I64, descriptor.word())),
            None => {
                if let Type::Base {
                    base: Base::Param(param),
                    ..
                } = ty
                {
                    return env
                        .get(&descriptor_name(param))
                        .map(|(value, _)| *value)
                        .ok_or_else(|| internal(format!("no type descriptor for `{param}`")));
                }
                // A composite: the description of `ty` with its type parameters as slots, and
                // the caller's descriptor of each (ADR 0068 D6).
                let params = type_params(ty);
                let slots: Vec<Type> = (0..params.len())
                    .map(|position| base(Base::Param(crate::describe::slot_name(position))))
                    .collect();
                let described = substitute(ty, &params, &slots);
                let index =
                    i64::try_from(self.emitter.types.of(&described)).map_err(crate::internal)?;
                let list = self.call("una_rt_list", &[])?;
                for param in &params {
                    let desc = env
                        .get(&descriptor_name(param))
                        .map(|(value, _)| *value)
                        .ok_or_else(|| internal(format!("no type descriptor for `{param}`")))?;
                    self.runtime("una_rt_push", &[list, desc]);
                }
                let index = self.builder.ins().iconst(types::I64, index);
                let desc = self.call("una_rt_descriptor_composite", &[index, list])?;
                self.release(list, Kind::List);
                Ok(desc)
            }
        }
    }

    /// The type descriptor of the type parameter that `atom` has as its type, when it has one.
    fn descriptor_of(&self, atom: &Atom, env: &Env) -> Option<Value> {
        match self.atom_type(atom, env) {
            Type::Base {
                base: Base::Param(param),
                ..
            } => env.get(&descriptor_name(&param)).map(|(value, _)| *value),
            _ => None,
        }
    }

    fn atom_type(&self, atom: &Atom, env: &Env) -> Type {
        match atom {
            Atom::Var(name) => env
                .get(name.as_str())
                .map_or_else(unknown, |(_, ty)| ty.clone()),
            Atom::Int(_) => base(Base::Int),
            Atom::Bool(_) => base(Base::Bool),
            Atom::Text(_) => base(Base::Text),
            Atom::F64(_) => base(Base::F64),
            Atom::Decimal { scale, .. } => base(Base::Decimal(*scale)),
            Atom::F32(_) => base(Base::F32),
            Atom::Unit => base(Base::Unit),
        }
    }

    /// The type of field `position` of `ctor`, a variant of the kind `ty`.
    fn field_type(&self, ty: &Type, ctor: &Ctor, position: usize) -> Type {
        let args: &[Type] = match ty {
            Type::Base {
                base: Base::Named { args, .. },
                ..
            } => args,
            _ => &[],
        };
        if ctor.kind.module == PRELUDE_MODULE {
            // JSON (`spec/23 §1`): `Json` holds itself through its arrays and objects.
            let named = |name: &str, args: Vec<Type>| Type::Base {
                base: Base::Named {
                    module: PRELUDE_MODULE.to_owned(),
                    name: name.to_owned(),
                    args,
                },
                refinements: Vec::new(),
                label: Label::public(),
            };
            let json = || named("Json", Vec::new());
            match (ctor.kind.name.as_str(), ctor.name.as_str()) {
                ("Json", "Boolean") => return base(Base::Bool),
                ("Json", "Number") | ("JsonError", _) => return base(Base::Int),
                ("Json", "Decimal" | "Str") => return base(Base::Text),
                ("Json", "Array") => return named("List", vec![json()]),
                ("Json", "Object") => return named("Map", vec![base(Base::Text), json()]),
                _ => {}
            }
            let arg = match (ctor.kind.name.as_str(), ctor.name.as_str()) {
                ("Option", "Some") | ("Result", "Ok") => args.first(),
                ("Result", "Err") => args.get(1),
                // The other kinds of the prelude that Core holds, as `Response` (`spec/24 §1`),
                // are read from their declarations below.
                _ => None,
            };
            if let Some(arg) = arg {
                return arg.clone();
            }
        }
        self.emitter
            .program
            .modules
            .iter()
            .filter(|module| module.path == ctor.kind.module)
            .flat_map(|module| &module.decls)
            .find_map(|decl| match decl {
                Decl::Data {
                    name,
                    params,
                    variants,
                    ..
                } if *name == ctor.kind.name => {
                    let field = variants.get(ctor.index)?.fields.get(position)?;
                    Some(substitute(&field.1, params, args))
                }
                _ => None,
            })
            .unwrap_or_else(unknown)
    }

    /// The word that the runtime function `function` makes of the literal `text`.
    fn literal(&mut self, function: &'static str, text: &str) -> Result<Value, CodegenError> {
        let id = self.emitter.bytes(text)?;
        let global = self
            .emitter
            .module
            .declare_data_in_func(id, self.builder.func);
        let address = self.builder.ins().symbol_value(types::I64, global);
        let len = i64::try_from(text.len()).map_err(crate::internal)?;
        let len = self.builder.ins().iconst(types::I64, len);
        self.call(function, &[address, len])
    }

    /// Calls the runtime function `name` with `args`; its result, if it has one.
    fn runtime(&mut self, name: &'static str, args: &[Value]) -> Option<Value> {
        let id = *self.emitter.runtime.get(name)?;
        let callee = self
            .emitter
            .module
            .declare_func_in_func(id, self.builder.func);
        let call = self.builder.ins().call(callee, args);
        self.builder.inst_results(call).first().copied()
    }

    /// The runtime function `name` applied to `args`, which returns a word.
    fn call(&mut self, name: &'static str, args: &[Value]) -> Result<Value, CodegenError> {
        self.runtime(name, args)
            .ok_or_else(|| internal(format!("`{name}` returns nothing")))
    }

    /// The borrowed word of field, item or captured value `index` of `object`.
    fn field(&mut self, object: Value, index: usize) -> Result<Value, CodegenError> {
        let index = i64::try_from(index).map_err(crate::internal)?;
        let index = self.builder.ins().iconst(types::I64, index);
        self.call("una_rt_field", &[object, index])
    }

    /// The index of the call site at `span`, for `una_rt_enter`.
    fn site(&mut self, span: Span) -> Value {
        let site = i64::try_from(self.emitter.sites.len()).unwrap_or(i64::MAX);
        self.emitter.sites.push(span);
        self.builder.ins().iconst(types::I64, site)
    }

    /// The owned result of the closure `callee` applied to `args`, a call that counts toward the
    /// maximum depth at `site`, and whether it failed.
    fn apply(
        &mut self,
        callee: Value,
        args: &[Value],
        site: Value,
    ) -> Result<(Value, Value), CodegenError> {
        let code = self.call("una_rt_closure_code", &[callee])?;
        let mut signature = self.emitter.module.make_signature();
        for _ in 0..=args.len() {
            signature.params.push(AbiParam::new(types::I64));
        }
        signature.returns.push(AbiParam::new(types::I64));
        signature.returns.push(AbiParam::new(types::I64));
        let signature = self.builder.import_signature(signature);
        let mut values = vec![callee];
        values.extend_from_slice(args);
        // A lambda owns its parameters: when the task stops before the call, the arguments
        // given to it are released here, and the stop comes back as a failure, [`STOPPED`]
        // with status 2, which the caller passes on as a callee's.
        let returned = self.builder.create_block();
        self.builder.append_block_param(returned, types::I64);
        self.builder.append_block_param(returned, types::I64);
        let stopping = self.call("una_rt_enter", &[site])?;
        let refused = self.builder.create_block();
        let calling = self.builder.create_block();
        self.builder
            .ins()
            .brif(stopping, refused, &[], calling, &[]);
        self.builder.switch_to_block(refused);
        for &arg in args {
            self.release(arg, Kind::Any);
        }
        let word = self.builder.ins().iconst(types::I64, STOPPED);
        let two = self.builder.ins().iconst(types::I64, 2);
        self.builder
            .ins()
            .jump(returned, &[word.into(), two.into()]);
        self.builder.switch_to_block(calling);
        let call = self.builder.ins().call_indirect(signature, code, &values);
        let results = self.builder.inst_results(call).to_vec();
        self.runtime("una_rt_leave", &[]);
        match (results.first(), results.get(1)) {
            (Some(&value), Some(&failed)) => {
                self.builder
                    .ins()
                    .jump(returned, &[value.into(), failed.into()]);
                self.builder.switch_to_block(returned);
                let results = self.builder.block_params(returned).to_vec();
                match (results.first(), results.get(1)) {
                    (Some(&value), Some(&failed)) => Ok((value, failed)),
                    _ => Err(internal("a closure lost its result")),
                }
            }
            _ => Err(internal("a closure lost its result")),
        }
    }

    /// Enters the call at `site` (`spec/11 §7`), before its arguments are given, or leaves the
    /// frame by the failure path when the runtime says the task stops.
    fn enter(&mut self, site: Value, scope: &Scope, catch: &Catch) -> Result<(), CodegenError> {
        let stopping = self.call("una_rt_enter", &[site])?;
        self.stop_or_fail(stopping, scope, catch);
        Ok(())
    }

    /// `set` of the prelude, by `function` (`una_rt_set` or `una_rt_set_owned`), at the call's
    /// site: an index outside the list aborts there with U8001, as the interpreter's fault does
    /// (#480, `spec/17 §7.3`), and a task stopped by it stops here. `slice` and `replace` call
    /// their runtime functions the same way, for their own `requires` (#693).
    fn set(
        &mut self,
        function: &'static str,
        args: &[Value],
        span: Span,
    ) -> Result<Value, CodegenError> {
        let site = self.site(span);
        self.sited(function, site, args)
    }

    /// The runtime function `function` applied to the call's `site`, then `args`, as [`set`]
    /// calls it: a task stopped by it stops here.
    ///
    /// [`set`]: Self::set
    fn sited(
        &mut self,
        function: &'static str,
        site: Value,
        args: &[Value],
    ) -> Result<Value, CodegenError> {
        let mut operands = vec![site];
        operands.extend_from_slice(args);
        let value = self.call(function, &operands)?;
        let stopped = self.call("una_rt_stopped", &[])?;
        self.stop_if(stopped);
        Ok(value)
    }

    /// 1 when the failure word `failure` is [`STOPPED`], else 0.
    fn stopped(&mut self, failure: Value) -> Value {
        let stopped = self
            .builder
            .ins()
            .icmp_imm_s(IntCC::Equal, failure, STOPPED);
        self.flag(stopped)
    }

    /// Goes on with [`STOPPED`] to `catch`, releasing the words of `scope` since it, when
    /// `failure` is the stop of the task; continues otherwise.
    fn pass_stop(&mut self, failure: Value, scope: &Scope, catch: &Catch) {
        let stopped = self.stopped(failure);
        self.stop_or_fail(stopped, scope, catch);
    }

    /// Leaves the frame by the failure path with [`STOPPED`] when `stopping` is not 0: the
    /// words of `scope` since `catch` was installed are released, as a failure releases them
    /// (`spec/19 §6` rule 4).
    fn stop_or_fail(&mut self, stopping: Value, scope: &Scope, catch: &Catch) {
        let failing = self.builder.create_block();
        let next = self.builder.create_block();
        self.builder.ins().brif(stopping, failing, &[], next, &[]);
        self.builder.switch_to_block(failing);
        let word = self.builder.ins().iconst(types::I64, STOPPED);
        self.fail(word, scope, catch);
        self.builder.switch_to_block(next);
    }

    /// A JSON call stopped at one of its check points (`stopping` not 0), by the memory budget
    /// or a cancellation: its value is released, and the frame left by the failure path.
    fn json_stop(&mut self, stopping: Value, value: Value, scope: &Scope, catch: &Catch) {
        let failing = self.builder.create_block();
        let next = self.builder.create_block();
        self.builder.ins().brif(stopping, failing, &[], next, &[]);
        self.builder.switch_to_block(failing);
        self.release(value, Kind::Any);
        let word = self.builder.ins().iconst(types::I64, STOPPED);
        self.fail(word, scope, catch);
        self.builder.switch_to_block(next);
    }

    /// `value`, once the task has been checked for a stop that a runtime call raised at its
    /// site, as a text that does not fit the memory budget does (`spec/11 §7`, #1223): a stopped
    /// task leaves here, before it uses the value.
    fn stopped_after(&mut self, value: Value) -> Result<Value, CodegenError> {
        let stopped = self.call("una_rt_stopped", &[])?;
        self.stop_if(stopped);
        Ok(value)
    }

    /// Stops the task at once when `stopping` is not 0: for an abort only (see `stop`).
    fn stop_if(&mut self, stopping: Value) {
        let next = self.builder.create_block();
        self.builder.ins().brif(stopping, self.stop, &[], next, &[]);
        self.builder.switch_to_block(next);
    }

    /// The one parameter of a join block.
    fn param(&self, block: Block) -> Result<Value, CodegenError> {
        self.builder
            .block_params(block)
            .first()
            .copied()
            .ok_or_else(|| internal("a join block lost its parameter"))
    }

    /// A Bool word from a comparison's flag.
    fn flag(&mut self, flag: Value) -> Value {
        self.builder.ins().uextend(types::I64, flag)
    }

    /// Calls the runtime function `name` on `value` when it is a handle.
    fn counting(&mut self, name: &'static str, value: Value, kind: Kind) {
        if !kind.counted() {
            return;
        }
        if kind.handle() {
            self.runtime(name, &[value]);
            return;
        }
        let counted = self.builder.create_block();
        let done = self.builder.create_block();
        let low = self.builder.ins().band_imm_s(value, 1);
        let even = self.builder.ins().icmp_imm_s(IntCC::Equal, low, 0);
        let nonzero = self.builder.ins().icmp_imm_s(IntCC::NotEqual, value, 0);
        let handle = self.builder.ins().band(even, nonzero);
        self.builder.ins().brif(handle, counted, &[], done, &[]);
        self.builder.switch_to_block(counted);
        self.runtime(name, &[value]);
        self.builder.ins().jump(done, &[]);
        self.builder.switch_to_block(done);
    }

    /// Adds a reference to `value`.
    fn retain(&mut self, value: Value, kind: Kind) {
        self.counting("una_rt_retain", value, kind);
    }

    /// Releases a reference to `value`.
    fn release(&mut self, value: Value, kind: Kind) {
        self.counting("una_rt_release", value, kind);
    }

    /// The position of the variant `value` in its kind: its own word for a variant without
    /// fields, else the runtime's.
    fn tag(&mut self, value: Value) -> Result<Value, CodegenError> {
        let immediate = self.builder.create_block();
        let object = self.builder.create_block();
        let done = self.join();
        let low = self.builder.ins().band_imm_s(value, 1);
        self.builder.ins().brif(low, immediate, &[], object, &[]);
        self.builder.switch_to_block(immediate);
        let tag = self.builder.ins().sshr_imm_s(value, 1);
        self.builder.ins().jump(done, &[tag.into()]);
        self.builder.switch_to_block(object);
        let tag = self.call("una_rt_tag", &[value])?;
        self.builder.ins().jump(done, &[tag.into()]);
        self.builder.switch_to_block(done);
        self.param(done)
    }

    /// A call of a function of `std.collections` whose body is native (`spec/28`, ADR 0119), on
    /// borrowed arguments at `span`: its owned result, or `None` for a function the module
    /// declares in Una.
    fn collections(
        &mut self,
        name: &str,
        args: &[Value],
        type_args: &[Type],
        span: Span,
        env: &Env,
    ) -> Result<Option<Value>, CodegenError> {
        let arg = |i: usize| {
            args.get(i)
                .copied()
                .ok_or_else(|| internal(format!("`{name}` lacks an argument")))
        };
        let made = match name {
            // O(1): an empty collection allocates no buffer.
            "empty_stack" | "empty_queue" => self.call("una_rt_seq_empty", &[])?,
            "empty_set" => self.call("una_rt_set_empty", &[])?,
            "lowest_first" | "highest_first" => {
                let highest = i64::from(name == "highest_first");
                let highest = self.builder.ins().iconst(types::I64, highest);
                self.call("una_rt_priority_empty", &[highest])?
            }
            // O(size / 64), within the memory budget, and U8001 for a negative size, at the call.
            "empty_bit_set" => self.set("una_rt_bit_set_empty", &[arg(0)?], span)?,
            // The conversions of a list (`spec/28 §10`): O(n), and O(n log n) for a set.
            "to_stack" | "to_queue" => {
                let stack = i64::from(name == "to_stack");
                let stack = self.builder.ins().iconst(types::I64, stack);
                self.call("una_rt_seq_of", &[arg(0)?, stack])?
            }
            "to_set" => self.call("una_rt_set_of", &[arg(0)?])?,
            // A new table under a key of its own (`spec/28 §1` rule 4), at the call: U6003 when
            // the run's table counter is exhausted. The run reads the process key before `main`
            // (ADR 0119 rule 9).
            "empty_hash_map" | "empty_hash_set" => {
                self.emitter.hashing = true;
                let set = i64::from(name == "empty_hash_set");
                let set = self.builder.ins().iconst(types::I64, set);
                self.set("una_rt_hash_empty", &[set], span)?
            }
            // O(n · |k|) expected, the keys hashed by the descriptor of the key type, the first
            // type argument, and compared within the memory budget.
            "to_hash_set" | "to_hash_map" => {
                self.emitter.hashing = true;
                let key = type_args
                    .first()
                    .ok_or_else(|| internal(format!("`{name}` lacks its key type")))?;
                let desc = self.descriptor(key, Constraint::Eq, env)?;
                let set = i64::from(name == "to_hash_set");
                let set = self.builder.ins().iconst(types::I64, set);
                self.set("una_rt_hash_of_list", &[set, desc, arg(0)?], span)?
            }
            _ => return Ok(None),
        };
        Ok(Some(made))
    }

    /// A call of the prelude (`spec/14 §1`), on borrowed arguments.
    fn prelude(
        &mut self,
        name: &str,
        args: &[Value],
        span: Span,
        scope: &Scope,
        catch: &Catch,
    ) -> Result<Value, CodegenError> {
        let arg = |i: usize| {
            args.get(i)
                .copied()
                .ok_or_else(|| internal(format!("`{name}` lacks an argument")))
        };
        let function = match name {
            "is_some" | "is_ok" => {
                let tag = self.tag(arg(0)?)?;
                let flag = self.builder.ins().icmp_imm_s(IntCC::Equal, tag, 0);
                return Ok(self.flag(flag));
            }
            "map" | "filter" | "first_where" | "fold" => {
                // The interpreter applies the function at the prelude call's site.
                let site = self.site(span);
                return self.higher_order(name, args, site, false, scope, catch);
            }
            // A failure of `parse_int` goes to `catch` (`spec/22 §4`).
            "parse_int" => {
                let t = arg(0)?;
                let failure = self.call("una_rt_parse_failure", &[t])?;
                let failed = self.builder.ins().icmp_imm_s(IntCC::NotEqual, failure, 0);
                self.check(failure, failed, scope, catch);
                return self.call("una_rt_parse_int", &[t]);
            }
            // A failure of `parse_decimal` goes to `catch`, as `parse_int`'s (`spec/29 §16`).
            "parse_decimal" => {
                let (t, scale) = (arg(0)?, arg(1)?);
                let failure = self.call("una_rt_parse_decimal_failure", &[t, scale])?;
                let failed = self.builder.ins().icmp_imm_s(IntCC::NotEqual, failure, 0);
                self.check(failure, failed, scope, catch);
                return self.call("una_rt_parse_decimal", &[t, scale]);
            }
            "as_decimal" => "una_rt_as_decimal",
            // Elaboration writes `json_decimal(x)` as an operation with the scale of `x`.
            "json_decimal" => {
                return Err(internal(
                    "`json_decimal` called without the scale of its argument",
                ));
            }
            "split" => "una_rt_split",
            "join" => "una_rt_join",
            "ends_with" => "una_rt_ends_with",
            "contains_text" => "una_rt_contains_text",
            "find" => "una_rt_find",
            // Outside their `requires`, they abort at the call's site as `set` does (#693).
            "replace" => return self.set("una_rt_replace", args, span),
            "slice" => return self.set("una_rt_slice", args, span),
            "to_upper" => "una_rt_to_upper",
            "to_lower" => "una_rt_to_lower",
            "chars" => "una_rt_chars",
            "trim_start" => "una_rt_trim_start",
            "trim_end" => "una_rt_trim_end",
            "bytes" => "una_rt_bytes",
            "abs" => "una_rt_abs",
            "min" => "una_rt_min",
            "max" => "una_rt_max",
            "is_digits" => "una_rt_is_digits",
            "starts_with" => "una_rt_starts_with",
            "trim" => "una_rt_trim",
            "to_int" => "una_rt_to_int",
            "is_empty" => "una_rt_is_empty",
            "first" => "una_rt_first",
            "at" => "una_rt_at",
            "append" => "una_rt_append",
            // Its comparisons check the memory budget at this call's site (#815).
            "contains" => return self.set("una_rt_contains_at", args, span),
            "set" => return self.set("una_rt_set", args, span),
            "sum" => "una_rt_sum",
            "or_else" => "una_rt_or_else",
            "empty_map" => "una_rt_map_empty",
            // JSON (`spec/23`): `parse_json` reports its failure as a host operation does.
            // Both check the memory budget at this call's site as they go: over it, the run
            // aborts with U6003 there, or the task stops.
            "parse_json" => {
                let site = self.site(span);
                let value = self.call("una_rt_parse_json", &[arg(0)?, site])?;
                let stopping = self.call("una_rt_json_stopped", &[])?;
                self.json_stop(stopping, value, scope, catch);
                let failed = self.call("una_rt_host_failed", &[])?;
                self.check(value, failed, scope, catch);
                return Ok(value);
            }
            "to_json" => {
                let site = self.site(span);
                let value = self.call("una_rt_to_json", &[arg(0)?, site])?;
                let stopping = self.call("una_rt_json_stopped", &[])?;
                self.json_stop(stopping, value, scope, catch);
                return Ok(value);
            }
            "quote_json" => "una_rt_quote_json",
            "is_json_number" => "una_rt_is_json_number",
            // The functions of `F64` (`spec/29`, ADR 0114 rule 3.9).
            "nan" => {
                let bits = self.builder.ins().iconst(
                    types::I64,
                    i64::from_ne_bytes(una_core::float::CANONICAL_NAN.to_ne_bytes()),
                );
                let word = self.call("una_rt_f64_literal", &[bits])?;
                self.retain(word, Kind::F64);
                return Ok(word);
            }
            "infinity" => {
                let bits = self.builder.ins().iconst(
                    types::I64,
                    i64::from_ne_bytes(f64::INFINITY.to_bits().to_ne_bytes()),
                );
                let word = self.call("una_rt_f64_literal", &[bits])?;
                self.retain(word, Kind::F64);
                return Ok(word);
            }
            "f64_from_bits" => "una_rt_f64_from_bits",
            // The functions of `F32` (ADR 0120 rule 5).
            "f32_from_bits" => "una_rt_f32_from_bits",
            "parse_f32" => {
                let t = arg(0)?;
                let failure = self.call("una_rt_parse_f64_failure", &[t])?;
                let failed = self.builder.ins().icmp_imm_s(IntCC::NotEqual, failure, 0);
                self.check(failure, failed, scope, catch);
                return self.call("una_rt_parse_f32", &[t]);
            }
            // A failure of `parse_f64` goes to `catch`, as `parse_int`'s does.
            "parse_f64" => {
                let t = arg(0)?;
                let failure = self.call("una_rt_parse_f64_failure", &[t])?;
                let failed = self.builder.ins().icmp_imm_s(IntCC::NotEqual, failure, 0);
                self.check(failure, failed, scope, catch);
                return self.call("una_rt_parse_f64", &[t]);
            }
            "as_f64" => "una_rt_as_f64",
            "json_number" => "una_rt_json_number",
            other => return Err(unsupported(format!("`{other}` of the prelude"))),
        };
        self.call(function, args)
    }

    /// `map`, `filter`, `first_where` and `fold`: a loop over the list's items that applies the
    /// function to each, in order.
    /// In edition 0.2 the function may fail (ADR 0062): its failure stops the loop, releases what
    /// the loop made, and goes to `catch`, after the effects already performed.
    /// Every call of the function gives it its arguments as owned: an item is retained
    /// for it, and `fold`'s state moves into it, which returns the next state; `initial_owned`
    /// says that the initial state moved into the loop already.
    fn higher_order(
        &mut self,
        name: &str,
        args: &[Value],
        site: Value,
        initial_owned: bool,
        scope: &Scope,
        catch: &Catch,
    ) -> Result<Value, CodegenError> {
        let (Some(&list), Some(&function)) = (args.first(), args.last()) else {
            return Err(internal(format!("`{name}` lacks an argument")));
        };
        let count = self.call("una_rt_count", &[list])?;
        // The loop carries the item's index and, for `fold`, the accumulator; `first_where`
        // leaves it with what it found.
        let head = self.builder.create_block();
        let body = self.builder.create_block();
        let exit = self.join();
        self.builder.append_block_param(head, types::I64);
        self.builder.append_block_param(head, types::I64);
        let zero = self.builder.ins().iconst(types::I64, 0);
        let (initial, out) = match name {
            "fold" => {
                let initial = args
                    .get(1)
                    .copied()
                    .ok_or_else(|| internal("`fold` lacks its initial value"))?;
                if !initial_owned {
                    self.retain(initial, Kind::Any);
                }
                (initial, None)
            }
            "first_where" => (self.builder.ins().iconst(types::I64, NONE), None),
            _ => {
                let out = self.call("una_rt_list", &[])?;
                (out, Some(out))
            }
        };
        self.builder
            .ins()
            .jump(head, &[zero.into(), initial.into()]);

        self.builder.switch_to_block(head);
        let params = self.builder.block_params(head).to_vec();
        let (Some(&index), Some(&carried)) = (params.first(), params.get(1)) else {
            return Err(internal("a loop lost its parameters"));
        };
        let more = self.builder.ins().icmp(IntCC::SignedLessThan, index, count);
        self.builder
            .ins()
            .brif(more, body, &[], exit, &[carried.into()]);

        self.builder.switch_to_block(body);
        let item = self.call("una_rt_field", &[list, index])?;
        // The function owns the item it is given, and the list keeps its own.
        self.retain(item, Kind::Any);
        let next_index = self.builder.ins().iadd_imm_s(index, 1);
        match name {
            "map" => {
                let (mapped, failed) = self.apply(function, &[item], site)?;
                self.stop_on_failure(mapped, failed, &[(carried, Kind::List)], scope, catch);
                self.runtime("una_rt_push", &[carried, mapped]);
                self.release(mapped, Kind::Any);
                self.builder
                    .ins()
                    .jump(head, &[next_index.into(), carried.into()]);
            }
            "filter" | "first_where" => {
                let (keep, failed) = self.apply(function, &[item], site)?;
                // `filter` owns the list it builds; `first_where` carries `None`, an immediate.
                self.stop_on_failure(keep, failed, &[(carried, Kind::Any)], scope, catch);
                let kept = self.builder.create_block();
                let next = self.builder.create_block();
                self.builder.ins().brif(keep, kept, &[], next, &[]);
                self.builder.switch_to_block(kept);
                if name == "filter" {
                    self.runtime("una_rt_push", &[carried, item]);
                    self.builder.ins().jump(next, &[]);
                } else {
                    let zero = self.builder.ins().iconst(types::I64, 0);
                    let found = self.call("una_rt_data", &[zero])?;
                    self.runtime("una_rt_push", &[found, item]);
                    self.builder.ins().jump(exit, &[found.into()]);
                }
                self.builder.switch_to_block(next);
                self.builder
                    .ins()
                    .jump(head, &[next_index.into(), carried.into()]);
            }
            _ => {
                // The state moves into the step, which releases it on a failure too.
                let (step, failed) = self.apply(function, &[carried, item], site)?;
                self.stop_on_failure(step, failed, &[], scope, catch);
                self.builder
                    .ins()
                    .jump(head, &[next_index.into(), step.into()]);
            }
        }

        self.builder.switch_to_block(exit);
        match out {
            Some(out) => Ok(out),
            None => self.param(exit),
        }
    }

    /// A dense function of the prelude (`spec/29 §17`, ADR 0120; design note
    /// `docs/plan/0120-s6-design.md`): its entry point `una_rt_<name>`, the call's site first,
    /// so a refusal of the memory budget (U6003) or a precondition that fails (U8001) stops
    /// there. A method that takes its receiver by `edit`, at the receiver's last use, takes it
    /// over through its `_owned` entry point, which updates it in place when it and its store
    /// are held once (D3, rule 31). `map`, `zip_map`, `update` and `fold` are loops
    /// ([`Self::dense_loop`], D6).
    #[allow(clippy::too_many_arguments)]
    fn dense_call(
        &mut self,
        name: &str,
        args: &[Atom],
        values: &[Value],
        span: Span,
        env: &Env,
        scope: &mut Scope,
        catch: &Catch,
        rest: &Expr,
    ) -> Result<Value, CodegenError> {
        let method = dense_method(name);
        if name.starts_with("dense_") && matches!(method, "map" | "zip_map" | "update" | "fold") {
            // The receiver of `update`, and the initial state of `fold` unless it is the value
            // folded over, which the loop still reads, move in at their last use.
            let moving = match method {
                "update" => args.first(),
                "fold" if values.get(1).is_some() && values.first() != values.get(1) => args.get(1),
                _ => None,
            };
            let moved = match moving {
                Some(atom) => self.moving(atom, env, scope, catch, rest),
                None => None,
            };
            if let Some(position) = moved {
                scope.remove(position);
            }
            let site = self.site(span);
            return self.dense_loop(method, values, site, moved.is_some(), scope, catch);
        }
        let mut runtime = format!("una_rt_{name}");
        if dense_edits(name)
            && let Some(position) = args
                .first()
                .and_then(|receiver| self.moving(receiver, env, scope, catch, rest))
        {
            scope.remove(position);
            runtime.push_str("_owned");
        }
        let function = self
            .emitter
            .runtime
            .get_key_value(runtime.as_str())
            .map(|(function, _)| *function)
            .ok_or_else(|| unsupported(format!("`{name}` of the prelude")))?;
        self.set(function, values, span)
    }

    /// `map`, `zip_map`, `update` and `fold` of a dense value (design note D6): a loop over its
    /// elements in row-major order that applies the function to each through the uniform word,
    /// as a function value that is not an inline lambda is (ADR 0120 rule 35). `map` and
    /// `zip_map` store each result in a builder of the receiver's kind and shape, whose store
    /// the first result chooses and reserves; `update` writes each into the receiver held once
    /// (rule 31), which `moved` says the caller gave up; `fold` carries its state, which
    /// `moved` says moved in. A failure of the function stops the loop, releases what it made,
    /// and goes to `catch`, as the prelude's loops over lists do (ADR 0062). O(size) calls.
    fn dense_loop(
        &mut self,
        method: &str,
        args: &[Value],
        site: Value,
        moved: bool,
        scope: &Scope,
        catch: &Catch,
    ) -> Result<Value, CodegenError> {
        let (Some(&dense), Some(&function)) = (args.first(), args.last()) else {
            return Err(internal(format!("`{method}` lacks an argument")));
        };
        let second = || {
            args.get(1)
                .copied()
                .ok_or_else(|| internal(format!("`{method}` lacks an argument")))
        };
        let (source, initial) = match method {
            "map" => (dense, self.sited("una_rt_dense_builder", site, &[dense])?),
            "zip_map" => {
                let other = second()?;
                let builder = self.sited("una_rt_dense_zip_builder", site, &[dense, other])?;
                (dense, builder)
            }
            "update" => {
                // The call takes a reference: the caller's when it moved in.
                if !moved {
                    self.retain(dense, Kind::Dense);
                }
                let owned = self.sited("una_rt_dense_unique", site, &[dense])?;
                (owned, owned)
            }
            _ => {
                let initial = second()?;
                if !moved {
                    self.retain(initial, Kind::Any);
                }
                (dense, initial)
            }
        };
        let count = self.call("una_rt_dense_count", &[source])?;
        let head = self.builder.create_block();
        let body = self.builder.create_block();
        let exit = self.join();
        self.builder.append_block_param(head, types::I64);
        self.builder.append_block_param(head, types::I64);
        let zero = self.builder.ins().iconst(types::I64, 0);
        self.builder
            .ins()
            .jump(head, &[zero.into(), initial.into()]);

        self.builder.switch_to_block(head);
        let params = self.builder.block_params(head).to_vec();
        let (Some(&index), Some(&carried)) = (params.first(), params.get(1)) else {
            return Err(internal("a loop lost its parameters"));
        };
        let more = self.builder.ins().icmp(IntCC::SignedLessThan, index, count);
        self.builder
            .ins()
            .brif(more, body, &[], exit, &[carried.into()]);

        self.builder.switch_to_block(body);
        // The function owns the element it is given: a new boxed `F64`, or the word retained.
        let item = self.call("una_rt_dense_item", &[source, index])?;
        let next_index = self.builder.ins().iadd_imm_s(index, 1);
        match method {
            "map" | "zip_map" => {
                let mut operands = vec![item];
                if method == "zip_map" {
                    let other = second()?;
                    operands.push(self.call("una_rt_dense_item", &[other, index])?);
                }
                let (mapped, failed) = self.apply(function, &operands, site)?;
                self.stop_on_failure(mapped, failed, &[(carried, Kind::Dense)], scope, catch);
                self.sited("una_rt_dense_push", site, &[carried, mapped])?;
                self.release(mapped, Kind::Any);
                self.builder
                    .ins()
                    .jump(head, &[next_index.into(), carried.into()]);
            }
            "update" => {
                let (new, failed) = self.apply(function, &[item], site)?;
                self.stop_on_failure(new, failed, &[(carried, Kind::Dense)], scope, catch);
                self.runtime("una_rt_dense_write", &[carried, index, new]);
                self.release(new, Kind::Any);
                self.builder
                    .ins()
                    .jump(head, &[next_index.into(), carried.into()]);
            }
            _ => {
                // The state moves into the step, which releases it on a failure too.
                let (step, failed) = self.apply(function, &[carried, item], site)?;
                self.stop_on_failure(step, failed, &[], scope, catch);
                self.builder
                    .ins()
                    .jump(head, &[next_index.into(), step.into()]);
            }
        }

        self.builder.switch_to_block(exit);
        let result = self.param(exit)?;
        if method != "update" {
            return Ok(result);
        }
        // The new value of the `edit` receiver, with the result, `unit` (`spec/17 §4`).
        let zero = self.builder.ins().iconst(types::I64, 0);
        let tuple = self.call("una_rt_data", &[zero])?;
        self.runtime("una_rt_push", &[tuple, result]);
        self.release(result, Kind::Dense);
        self.runtime("una_rt_push", &[tuple, zero]);
        Ok(tuple)
    }

    /// The owned result of `prim` on `args`, whose operands and result `width` says are proven
    /// to fit a tagged word (`range`).
    /// Continues when `failed` is 0; otherwise releases `owned`, what a loop of the prelude
    /// made, and fails with `value`.
    fn stop_on_failure(
        &mut self,
        value: Value,
        failed: Value,
        owned: &[(Value, Kind)],
        scope: &Scope,
        catch: &Catch,
    ) {
        let failing = self.builder.create_block();
        let next = self.builder.create_block();
        self.builder.ins().brif(failed, failing, &[], next, &[]);
        self.builder.switch_to_block(failing);
        for &(word, kind) in owned {
            self.release(word, kind);
        }
        self.fail(value, scope, catch);
        self.builder.switch_to_block(next);
    }

    /// `prim` on values of a constrained type parameter, whose descriptor comes first in
    /// `operands` (`spec/20 §7`).
    fn generic_prim(
        &mut self,
        prim: Prim,
        operands: &[Value],
        span: Span,
    ) -> Result<Value, CodegenError> {
        let cc = match prim {
            Prim::Convert(_) => return self.call("una_rt_generic_to_int", operands),
            // Interpolation, by the type that instantiates the parameter (ADR 0068), at its
            // site, where a text that does not fit the memory budget aborts the run (`spec/11
            // §7`, #1223).
            Prim::ToText => {
                let mut sited = vec![self.site(span)];
                sited.extend_from_slice(operands);
                let text = self.call("una_rt_generic_write", &sited)?;
                return self.stopped_after(text);
            }
            Prim::Eq | Prim::Ne => {
                // Within the memory budget, at the comparison's site (#815).
                let equal = self.set("una_rt_generic_equal_at", operands, span)?;
                return Ok(if prim == Prim::Eq {
                    equal
                } else {
                    self.builder.ins().bxor_imm_s(equal, 1)
                });
            }
            Prim::Lt => IntCC::SignedLessThan,
            Prim::Le => IntCC::SignedLessThanOrEqual,
            Prim::Gt => IntCC::SignedGreaterThan,
            Prim::Ge => IntCC::SignedGreaterThanOrEqual,
            _ => {
                return Err(internal(format!(
                    "`{}` on a type parameter, which its constraint never allows",
                    prim.as_str()
                )));
            }
        };
        let order = self.call("una_rt_generic_compare", operands)?;
        let flag = self.builder.ins().icmp_imm_s(cc, order, 0);
        Ok(self.flag(flag))
    }

    fn prim(
        &mut self,
        prim: Prim,
        args: &[(Value, Kind)],
        width: Width,
        span: Span,
    ) -> Result<Value, CodegenError> {
        let first = args.first().copied();
        let second = args.get(1).copied();
        let operand = |i: Option<(Value, Kind)>| {
            i.ok_or_else(|| internal(format!("`{}` lacks an operand", prim.as_str())))
        };
        // An `F64` reaches only its own operations, equality, interpolation and `to_decimal`:
        // integer
        // arithmetic or a word comparison on its handle would compute on an address.
        if !matches!(
            prim,
            Prim::Float(_)
                | Prim::Float32(_)
                | Prim::Eq
                | Prim::Ne
                | Prim::ToText
                | Prim::Decimal(una_core::DecimalOp::FromF64(_))
        ) && args
            .iter()
            .any(|(_, kind)| matches!(kind, Kind::F64 | Kind::F32))
        {
            return Err(internal(format!("`{}` on a float", prim.as_str())));
        }
        match prim {
            // A conversion keeps the value: a narrowing one carries an `overflow` obligation that
            // proves it fits (`spec/18 §4`).
            Prim::Convert(_) => {
                let (a, kind) = operand(first)?;
                // `to_int()` on a type parameter needs its type descriptor (`spec/20 §7`).
                if kind == Kind::Any {
                    return Err(unsupported("`to_int()` on a type parameter (spec/20 §7)"));
                }
                self.retain(a, kind);
                Ok(a)
            }
            // Lowered with their arguments' ownership, in `rhs`.
            Prim::Map(op) => Err(internal(format!("map `{}` out of place", op.as_str()))),
            // Every operation of `F64` is the runtime's, over boxed values (ADR 0114 rule 9.6):
            // the same routines of `una-core` as the interpreter's, so the bits agree.
            Prim::Float(op) => {
                let code = una_core::FloatOp::ALL
                    .iter()
                    .position(|x| *x == op)
                    .and_then(|i| i64::try_from(i).ok())
                    .ok_or_else(|| internal("an unknown operation of `F64`"))?;
                let code = self.builder.ins().iconst(types::I64, code);
                let zero = self.builder.ins().iconst(types::I64, 0);
                let word = |i: usize| args.get(i).map_or(zero, |(value, _)| *value);
                let (a, b, c) = (word(0), word(1), word(2));
                self.call("una_rt_f64_op", &[code, a, b, c])
            }
            // The shape functions of a dense value (`spec/29 §17`, ADR 0120 rules 33 and 38), by
            // the runtime: an axis's length, -1 for the rank, -2 for the number of elements. An
            // `Lu[T]` of size n holds n + 1 rows of n, so its size is its columns.
            Prim::Shape(fun) => {
                let (a, _) = operand(first)?;
                let axis = match fun {
                    una_core::ShapeFun::Rows => 0,
                    una_core::ShapeFun::Columns | una_core::ShapeFun::Size => 1,
                    una_core::ShapeFun::Rank => -1,
                    una_core::ShapeFun::Count => -2,
                };
                let axis = self.builder.ins().iconst(types::I64, axis);
                self.call("una_rt_dense_shape", &[a, axis])
            }
            // Every operation of `Decimal` is the runtime's, on coefficients, with the
            // operation's static scales, and its conversions with `F64` on boxed values
            // (`spec/29 §16`).
            Prim::Decimal(op) => {
                let (index, packed) = op.code();
                let mut words = vec![
                    self.builder.ins().iconst(types::I64, index),
                    self.builder.ins().iconst(types::I64, packed),
                ];
                // `to_text` and `json_decimal` take one operand: the second word is their
                // site, where a text that does not fit the memory budget aborts the run
                // (`spec/11 §7`, #1223); a task stopped by it stops after the call.
                let writes = matches!(
                    op,
                    una_core::DecimalOp::ToText(_) | una_core::DecimalOp::ToJson(_)
                );
                for i in 0..3 {
                    words.push(match args.get(i) {
                        Some((word, _)) => *word,
                        None if writes && i == 1 => self.site(span),
                        None => self.builder.ins().iconst(types::I64, 1),
                    });
                }
                let value = self.call("una_rt_decimal", &words)?;
                if writes {
                    return self.stopped_after(value);
                }
                Ok(value)
            }
            // Lowered with their receiver's ownership, in `rhs`.
            Prim::Seq(_)
            | Prim::Set(_)
            | Prim::HashMap(_)
            | Prim::HashSet(_)
            | Prim::Priority(_)
            | Prim::BitSet(_) => Err(internal(format!(
                "collection `{}` out of place",
                prim.as_str()
            ))),
            // The operations of `F32`, over its tagged words (ADR 0120 rules 9 and 10): the
            // routines of `una_core::float32`, as in the interpreter.
            Prim::Float32(op) => {
                let code = una_core::FloatOp::ALL
                    .iter()
                    .position(|x| *x == op)
                    .and_then(|i| i64::try_from(i).ok())
                    .ok_or_else(|| internal("an unknown operation of `F32`"))?;
                let code = self.builder.ins().iconst(types::I64, code);
                let zero = self.builder.ins().iconst(types::I64, 0);
                let word = |i: usize| args.get(i).map_or(zero, |(value, _)| *value);
                let (a, b, c) = (word(0), word(1), word(2));
                self.call("una_rt_f32_op", &[code, a, b, c])
            }
            Prim::Fixed(op, int) => {
                let a = operand(first)?.0;
                let b = match second {
                    Some((b, _)) => b,
                    None => self.builder.ins().iconst(types::I64, 1),
                };
                let op = una_core::FixedOp::ALL
                    .iter()
                    .position(|x| *x == op)
                    .and_then(|i| i64::try_from(i).ok())
                    .ok_or_else(|| internal("an unknown fixed-width method"))?;
                let int = una_core::IntType::ALL
                    .iter()
                    .position(|x| *x == int)
                    .and_then(|i| i64::try_from(i).ok())
                    .ok_or_else(|| internal("an unknown fixed-width type"))?;
                let op = self.builder.ins().iconst(types::I64, op);
                let int = self.builder.ins().iconst(types::I64, int);
                self.call("una_rt_fixed", &[op, int, a, b])
            }
            Prim::Add | Prim::Sub | Prim::Mul => {
                let (a, b) = (operand(first)?.0, operand(second)?.0);
                if width.operands && width.result {
                    return Ok(self.unchecked(prim, a, b));
                }
                self.arithmetic(prim, a, b)
            }
            Prim::Div => {
                let (a, b) = (operand(first)?.0, operand(second)?.0);
                self.call("una_rt_int_div", &[a, b])
            }
            Prim::Mod => {
                let (a, b) = (operand(first)?.0, operand(second)?.0);
                self.call("una_rt_int_mod", &[a, b])
            }
            Prim::Neg => {
                let a = operand(first)?.0;
                self.call("una_rt_int_neg", &[a])
            }
            Prim::Not => {
                let a = operand(first)?.0;
                Ok(self.builder.ins().bxor_imm_s(a, 1))
            }
            Prim::Eq | Prim::Ne | Prim::Lt | Prim::Le | Prim::Gt | Prim::Ge => {
                let (a, kind) = operand(first)?;
                let b = operand(second)?.0;
                let cc = match prim {
                    Prim::Eq => IntCC::Equal,
                    Prim::Ne => IntCC::NotEqual,
                    Prim::Lt => IntCC::SignedLessThan,
                    Prim::Le => IntCC::SignedLessThanOrEqual,
                    Prim::Gt => IntCC::SignedGreaterThan,
                    _ => IntCC::SignedGreaterThanOrEqual,
                };
                let equality = matches!(prim, Prim::Eq | Prim::Ne);
                match kind {
                    Kind::Int if width.operands => {
                        // Both are tagged words, which order as their integers.
                        let flag = self.builder.ins().icmp(cc, a, b);
                        Ok(self.flag(flag))
                    }
                    Kind::Int => self.compare_ints(cc, a, b),
                    Kind::Bool | Kind::Unit => {
                        let flag = self.builder.ins().icmp(cc, a, b);
                        Ok(self.flag(flag))
                    }
                    Kind::Text if !equality => {
                        let order = self.call("una_rt_text_compare", &[a, b])?;
                        let flag = self.builder.ins().icmp_imm_s(cc, order, 0);
                        Ok(self.flag(flag))
                    }
                    // Variants order by their position in the kind (`spec/03 §1`).
                    Kind::Data if !equality => {
                        let (x, y) = (self.tag(a)?, self.tag(b)?);
                        let flag = self.builder.ins().icmp(cc, x, y);
                        Ok(self.flag(flag))
                    }
                    _ if equality => {
                        // `equal` is 1 when equal; as an order, 0 when equal. It compares within
                        // the memory budget, at the comparison's site (#815).
                        let equal = self.set("una_rt_equal_at", &[a, b], span)?;
                        let order = self.builder.ins().bxor_imm_s(equal, 1);
                        let flag = self.builder.ins().icmp_imm_s(cc, order, 0);
                        Ok(self.flag(flag))
                    }
                    _ => Err(unsupported("this comparison")),
                }
            }
            Prim::Len => {
                let (a, kind) = operand(first)?;
                match kind {
                    Kind::Text => self.call("una_rt_text_len", &[a]),
                    Kind::List => self.call("una_rt_list_len", &[a]),
                    Kind::Map => self.call("una_rt_map_len", &[a]),
                    // An array's length, its one axis (`spec/29 §17`).
                    Kind::Dense => {
                        let axis = self.builder.ins().iconst(types::I64, 0);
                        self.call("una_rt_dense_shape", &[a, axis])
                    }
                    // O(1) for every collection with a native layout (`spec/28`).
                    Kind::Coll => self.call("una_rt_collection_len", &[a]),
                    _ => Err(unsupported("this length")),
                }
            }
            Prim::Concat => {
                let mut parts = args.iter();
                let Some(&(mut text, _)) = parts.next() else {
                    let empty = self.literal("una_rt_text_literal", "")?;
                    self.retain(empty, Kind::Text);
                    return Ok(empty);
                };
                let mut owned = false;
                for &(part, _) in parts {
                    let joined = self.call("una_rt_text_concat", &[text, part])?;
                    if owned {
                        self.release(text, Kind::Text);
                    }
                    text = joined;
                    owned = true;
                }
                if !owned {
                    self.retain(text, Kind::Text);
                }
                Ok(text)
            }
            Prim::ToText => {
                let (a, kind) = operand(first)?;
                match kind {
                    // At its site, where a text that does not fit the memory budget aborts the
                    // run (`spec/11 §7`, #1223).
                    Kind::Int => {
                        let site = self.site(span);
                        let text = self.call("una_rt_int_text", &[site, a])?;
                        self.stopped_after(text)
                    }
                    Kind::Bool => self.call("una_rt_bool_text", &[a]),
                    Kind::F64 => {
                        let code = una_core::FloatOp::ALL
                            .iter()
                            .position(|x| *x == una_core::FloatOp::ToText)
                            .and_then(|i| i64::try_from(i).ok())
                            .ok_or_else(|| internal("an unknown operation of `F64`"))?;
                        let code = self.builder.ins().iconst(types::I64, code);
                        let zero = self.builder.ins().iconst(types::I64, 0);
                        self.call("una_rt_f64_op", &[code, a, zero, zero])
                    }
                    Kind::F32 => {
                        let code = una_core::FloatOp::ALL
                            .iter()
                            .position(|x| *x == una_core::FloatOp::ToText)
                            .and_then(|i| i64::try_from(i).ok())
                            .ok_or_else(|| internal("an unknown operation of `F32`"))?;
                        let code = self.builder.ins().iconst(types::I64, code);
                        let zero = self.builder.ins().iconst(types::I64, 0);
                        self.call("una_rt_f32_op", &[code, a, zero, zero])
                    }
                    Kind::Text => {
                        self.retain(a, Kind::Text);
                        Ok(a)
                    }
                    _ => Err(unsupported("this value as text")),
                }
            }
        }
    }

    /// `a op b` over tagged words whose result is proven to fit one: plain machine arithmetic,
    /// with its tag bit forced ([`unchecked_word`]).
    fn unchecked(&mut self, prim: Prim, a: Value, b: Value) -> Value {
        unchecked_word(&mut self.builder, prim, a, b)
    }

    /// `a op b` over integer words: computed inline when both are tagged and the result fits
    /// 63 bits, else by the runtime, which promotes to arbitrary precision (NB-3).
    fn arithmetic(&mut self, prim: Prim, a: Value, b: Value) -> Result<Value, CodegenError> {
        let fast = self.builder.create_block();
        let slow = self.builder.create_block();
        let done = self.join();
        let both = self.builder.ins().band(a, b);
        let tagged = self.builder.ins().band_imm_s(both, 1);
        self.builder.ins().brif(tagged, fast, &[], slow, &[]);

        self.builder.switch_to_block(fast);
        // With `a = 2x + 1` and `b = 2y + 1`, each result is computed as a tagged word.
        let (result, overflow) = match prim {
            Prim::Add => {
                let untagged = self.builder.ins().iadd_imm_s(a, -1);
                self.builder.ins().sadd_overflow(untagged, b)
            }
            Prim::Sub => {
                let (difference, overflow) = self.builder.ins().ssub_overflow(a, b);
                (self.builder.ins().iadd_imm_s(difference, 1), overflow)
            }
            _ => {
                let x = self.builder.ins().sshr_imm_s(a, 1);
                let twice_y = self.builder.ins().iadd_imm_s(b, -1);
                let (product, overflow) = self.builder.ins().smul_overflow(x, twice_y);
                (self.builder.ins().iadd_imm_s(product, 1), overflow)
            }
        };
        self.builder
            .ins()
            .brif(overflow, slow, &[], done, &[result.into()]);

        self.builder.switch_to_block(slow);
        let name = match prim {
            Prim::Add => "una_rt_int_add",
            Prim::Sub => "una_rt_int_sub",
            _ => "una_rt_int_mul",
        };
        let value = self.call(name, &[a, b])?;
        self.builder.ins().jump(done, &[value.into()]);

        self.builder.switch_to_block(done);
        self.param(done)
    }

    /// The comparison `cc` of two integer words, as a `Bool` word: of the words themselves when
    /// both are tagged, since tagging keeps the order, else of the runtime's order.
    fn compare_ints(&mut self, cc: IntCC, a: Value, b: Value) -> Result<Value, CodegenError> {
        let fast = self.builder.create_block();
        let slow = self.builder.create_block();
        let done = self.join();
        let both = self.builder.ins().band(a, b);
        let tagged = self.builder.ins().band_imm_s(both, 1);
        self.builder.ins().brif(tagged, fast, &[], slow, &[]);

        self.builder.switch_to_block(fast);
        let flag = self.builder.ins().icmp(cc, a, b);
        let value = self.flag(flag);
        self.builder.ins().jump(done, &[value.into()]);

        self.builder.switch_to_block(slow);
        let order = self.call("una_rt_int_compare", &[a, b])?;
        let flag = self.builder.ins().icmp_imm_s(cc, order, 0);
        let value = self.flag(flag);
        self.builder.ins().jump(done, &[value.into()]);

        self.builder.switch_to_block(done);
        self.param(done)
    }
}

/// Whether a primitive's operands, and its result, are proven to fit a tagged word.
#[derive(Clone, Copy, Debug)]
struct Width {
    operands: bool,
    result: bool,
}

/// The level that redacts a value labeled `label` in a trace, when it is above `public`.
fn redaction(label: &Label) -> Option<&'static str> {
    (!label.is_public()).then(|| describe::level(label.level))
}

/// A binder's source name: Core renames a binder that shares a name with another, and `k%2` is
/// keyed as `k` in traces (`spec/11 §5`).
fn source_name(name: &str) -> String {
    match name.split_once('%') {
        Some((source, _)) if !source.is_empty() => source.to_owned(),
        _ => name.to_owned(),
    }
}

/// The word of `None`, the variant of `Option` at position 1, without fields.
const NONE: i64 = (1 << 1) | 1;

/// The bindings other words borrow from in `expr`: the scrutinees of `case`, the tuples of
/// `let (...)`, and the lists and states that loops visit, in it and in the lambdas it makes.
fn borrowed_from(expr: &Expr, out: &mut BTreeSet<String>) {
    let add = |atom: &Atom, out: &mut BTreeSet<String>| {
        if let Atom::Var(name) = atom {
            out.insert(name.clone());
        }
    };
    match expr {
        Expr::Case {
            scrutinee,
            arms,
            otherwise,
        } => {
            add(scrutinee, out);
            for arm in arms {
                borrowed_from(&arm.body, out);
            }
            if let Some(otherwise) = otherwise {
                borrowed_from(otherwise, out);
            }
        }
        Expr::LetTuple { value, body, .. } => {
            add(value, out);
            borrowed_from(body, out);
        }
        Expr::Loop {
            source,
            body,
            on_stop,
            ..
        } => {
            match source.as_ref() {
                LoopSource::List(list) => add(list, out),
                LoopSource::Until { cond, .. } => borrowed_from(cond, out),
                LoopSource::Range { .. } | LoopSource::Ever { .. } => {}
            }
            borrowed_from(body, out);
            if let Some(on_stop) = on_stop {
                borrowed_from(&on_stop.body, out);
            }
        }
        Expr::Let { value, body, .. } => {
            if let Bound::Expr(inner) = value.as_ref() {
                borrowed_from(inner, out);
            }
            borrowed_from(body, out);
        }
        Expr::If {
            then, otherwise, ..
        } => {
            borrowed_from(then, out);
            borrowed_from(otherwise, out);
        }
        Expr::Handle { body, handler, .. } => {
            borrowed_from(body, out);
            borrowed_from(handler, out);
        }
        Expr::Attempt { body, .. }
        | Expr::With { body, .. }
        | Expr::Within { body, .. }
        | Expr::Tasks { body, .. }
        | Expr::Quiet { body, .. } => {
            borrowed_from(body, out);
        }
        Expr::Together { .. }
        | Expr::TogetherFor { .. }
        | Expr::FirstOf { .. }
        | Expr::Start { .. } => {
            // A task's captures are never moved: every name the block reads stays pinned
            // (#162).
            vars(expr, out);
        }
        Expr::Atom { .. } | Expr::Fail { .. } | Expr::Exit { .. } | Expr::Unreachable { .. } => {}
    }
}

/// The failure word of a stopped task (`spec/19 §6` rule 4): a task that is cancelled, by a
/// failed sibling or a `within` whose deadline passed (`spec/27 §5`), or that aborts, leaves
/// every frame by the path a failure takes, so each frame releases what it owns, and a frame
/// that catches failures passes this word on.
///
/// **Why `-1` is never a real failure.** A failure is a value of the function's error kind,
/// which is always a sum kind (`fails` names one, U2002), and a value of a sum kind is one of
/// two words, by the representation of `una-rt` (`values`):
/// - a variant without fields: the immediate `(index << 1) | 1`, with `index`, its position
///   in its kind, at least 0, so the word is at least 1 ([`Lowerer::construct`]);
/// - a variant with fields: a handle, an even word other than 0 (`values::is_handle`).
///
/// `-1` is odd, so no handle, and below 1, so no variant without fields. The integer `-1`,
/// whose small-integer word is also `-1`, is never a failure: no error kind is `Int`. Debug
/// builds assert it where a failure word is made: [`Lowerer::construct`] for variants, and
/// `una_rt::host::failed` for the failures of host operations.
const STOPPED: i64 = -1;

/// The trap of a branch that verification proves unreachable.
const TRAP: cranelift_codegen::ir::TrapCode = cranelift_codegen::ir::TrapCode::unwrap_user(1);

/// `a op b`, for `+`, `-` or `*`, at a site where the range facts prove both operands and the
/// result in the tagged range (ADR 0055 NB-3, `range.rs`): wrapping 64-bit machine arithmetic on
/// the tagged words, with no tag test, no overflow test and no promotion, and bit 0 of the
/// result forced to 1 (#2245), one `bor_imm` per site.
///
/// **Why forcing the tag changes no correct result.** A tagged `Int` `v` is the word
/// `2v + 1` (`una-rt`, `values.rs`): bit 0 is 1, and a handle is an even word other than 0.
/// When both operands are tagged, `a = 2x + 1` and `b = 2y + 1`, and the word computed is
///
/// - `+`: `(a - 1) + b = 2(x + y) + 1`;
/// - `-`: `(a - b) + 1 = 2(x - y) + 1`;
/// - `*`: `(a >> 1) * (b - 1) + 1 = x * 2y + 1 = 2xy + 1`, where `a >> 1` is arithmetic and
///   gives `x` exactly.
///
/// Each is computed modulo 2^64, and 2^64 is even, so the parity of each is that of the exact
/// value: odd. Bit 0 is already 1, and `r | 1 = r`. This holds for any two tagged operands,
/// whether or not the result fact holds; when it holds the word is also exactly the tagged word
/// of `x op y`, since `2(x op y) + 1` then lies in `[-2^63, 2^63)` and nothing wraps. So the
/// `bor_imm` changes the word only when an operand is not a tagged word, which a correct operand
/// fact excludes: a handle (an `Int` promoted past the tagged range, against its proven range)
/// or 0. Then it turns what could be an even word, a handle naming a live object of the heap,
/// into an odd word, an `Int` immediate: a wrong fact costs a wrong `Int` value at worst, as
/// ADR 0112 EL-2 bounds a wrong proof, never a word that the runtime would read, retain or
/// release as an object.
fn unchecked_word(builder: &mut FunctionBuilder<'_>, prim: Prim, a: Value, b: Value) -> Value {
    let word = match prim {
        Prim::Add => {
            let untagged = builder.ins().iadd_imm_s(a, -1);
            builder.ins().iadd(untagged, b)
        }
        Prim::Sub => {
            let difference = builder.ins().isub(a, b);
            builder.ins().iadd_imm_s(difference, 1)
        }
        _ => {
            let x = builder.ins().sshr_imm_s(a, 1);
            let twice_y = builder.ins().iadd_imm_s(b, -1);
            let product = builder.ins().imul(x, twice_y);
            builder.ins().iadd_imm_s(product, 1)
        }
    };
    builder.ins().bor_imm_s(word, 1)
}

#[cfg(test)]
mod tests {
    use una_core::IntType;

    use super::*;

    fn named(module: &str, name: &str) -> Type {
        base(Base::Named {
            module: module.to_owned(),
            name: name.to_owned(),
            args: Vec::new(),
        })
    }

    /// The function `(a, b) -> a op b` of [`unchecked_word`], or, with `operands`, the same
    /// function of no parameters over those two constant words; with `optimized`, after the
    /// mid-end of the instruction set that `una build` compiles for (opt level `speed`).
    fn unchecked_function(
        prim: Prim,
        operands: Option<(i64, i64)>,
        optimized: bool,
    ) -> cranelift_codegen::ir::Function {
        use cranelift_codegen::Context;
        use cranelift_codegen::control::ControlPlane;
        use cranelift_codegen::ir::{Function, Signature, UserFuncName};
        use cranelift_frontend::FunctionBuilderContext;

        let isa = crate::isa().unwrap();
        let mut signature = Signature::new(isa.default_call_conv());
        if operands.is_none() {
            signature.params.push(AbiParam::new(types::I64));
            signature.params.push(AbiParam::new(types::I64));
        }
        signature.returns.push(AbiParam::new(types::I64));
        let mut func = Function::with_name_signature(UserFuncName::user(0, 0), signature);
        let mut context = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut func, &mut context);
            let block = builder.create_block();
            builder.append_block_params_for_function_params(block);
            builder.switch_to_block(block);
            builder.seal_block(block);
            let (a, b) = match operands {
                Some((a, b)) => (
                    builder.ins().iconst(types::I64, a),
                    builder.ins().iconst(types::I64, b),
                ),
                None => {
                    let params = builder.block_params(block);
                    (params[0], params[1])
                }
            };
            let result = unchecked_word(&mut builder, prim, a, b);
            builder.ins().return_(&[result]);
            builder.finalize(isa.frontend_config());
        }
        if !optimized {
            return func;
        }
        let mut context = Context::for_function(func);
        context
            .optimize(isa.as_ref(), &mut ControlPlane::default())
            .unwrap();
        context.func
    }

    /// The word `a op b` of [`unchecked_word`] on two constant words, as Cranelift's own
    /// constant folding computes it in the mid-end of `una build`: the 64-bit semantics of the
    /// instructions it emits, without running them.
    fn unchecked_constant(prim: Prim, a: i64, b: i64) -> i64 {
        use cranelift_codegen::ir::InstructionData;
        let func = unchecked_function(prim, Some((a, b)), true);
        let entry = func.layout.entry_block().unwrap();
        let last = func.layout.last_inst(entry).unwrap();
        let returned = func.dfg.inst_args(last)[0];
        let defined = func.dfg.value_def(returned).unwrap_inst();
        match func.dfg.insts[defined] {
            InstructionData::UnaryImm { imm, .. } => imm.bits(),
            _ => panic!("not folded to a constant:\n{}", func.display()),
        }
    }

    const PRIMS: [Prim; 3] = [Prim::Add, Prim::Sub, Prim::Mul];

    /// The instructions of `func`'s entry block, without comments: the right-hand side of each
    /// definition, then the `return`.
    fn instructions(func: &cranelift_codegen::ir::Function) -> Vec<String> {
        func.display()
            .to_string()
            .lines()
            .map(|line| line.split(" ; ").next().unwrap_or(line).trim())
            .filter_map(|line| {
                line.split_once(" = ")
                    .map(|(_, rhs)| rhs.to_owned())
                    .or_else(|| line.starts_with("return").then(|| line.to_owned()))
            })
            .collect()
    }

    #[test]
    fn an_unchecked_operation_forces_the_tag_bit_of_its_result_2245() {
        // The lowering of `+`, `-` and `*` at a machine-width site ends with `bor` 1 on the
        // arithmetic's word: one instruction per site, which the mid-end of `una build` keeps.
        let expected: [(Prim, &[&str]); 3] = [
            (
                Prim::Add,
                &[
                    "iconst.i64 -1",
                    "iadd v0, v2",
                    "iadd v3, v1",
                    "iconst.i64 1",
                    "bor v4, v5",
                    "return v6",
                ],
            ),
            (
                Prim::Sub,
                &[
                    "isub v0, v1",
                    "iconst.i64 1",
                    "iadd v2, v3",
                    "iconst.i64 1",
                    "bor v4, v5",
                    "return v6",
                ],
            ),
            (
                Prim::Mul,
                &[
                    "iconst.i64 1",
                    "sshr v0, v2",
                    "iconst.i64 -1",
                    "iadd v1, v4",
                    "imul v3, v5",
                    "iconst.i64 1",
                    "iadd v6, v7",
                    "iconst.i64 1",
                    "bor v8, v9",
                    "return v10",
                ],
            ),
        ];
        for (prim, lines) in expected {
            let lowered = unchecked_function(prim, None, false);
            assert_eq!(
                instructions(&lowered),
                lines,
                "{prim:?}:\n{}",
                lowered.display()
            );
            let optimized = unchecked_function(prim, None, true);
            let after = instructions(&optimized);
            let ors = after.iter().filter(|line| line.starts_with("bor ")).count();
            let returned = after[after.len() - 2].clone();
            assert!(
                ors == 1 && returned.starts_with("bor "),
                "{prim:?}: the mid-end dropped the tag:\n{}",
                optimized.display()
            );
        }
    }

    /// The tagged word of `v`, as `una-rt` makes it.
    fn tagged(v: i64) -> i64 {
        (v << 1) | 1
    }

    #[test]
    fn the_forced_tag_changes_no_result_in_the_tagged_range_2245() {
        // Every pair of operands from the edges of the tagged range, and around 0, whose exact
        // result is in range too: the word is the tagged word of the exact result, bit for bit.
        const LOW: i64 = -(1 << 62);
        const HIGH: i64 = (1 << 62) - 1;
        let edges = [
            LOW,
            LOW + 1,
            LOW / 2,
            -(1 << 31),
            -3,
            -2,
            -1,
            0,
            1,
            2,
            3,
            (1 << 31) - 1,
            HIGH / 2,
            HIGH / 2 + 1,
            HIGH - 1,
            HIGH,
        ];
        let mut compared = 0;
        for prim in PRIMS {
            for x in edges {
                for y in edges {
                    let (x, y) = (i128::from(x), i128::from(y));
                    let exact = match prim {
                        Prim::Add => x + y,
                        Prim::Sub => x - y,
                        _ => x * y,
                    };
                    let Ok(exact) = i64::try_from(exact) else {
                        continue;
                    };
                    if !(LOW..=HIGH).contains(&exact) {
                        continue;
                    }
                    let (a, b) = (tagged(x as i64), tagged(y as i64));
                    assert_eq!(
                        unchecked_constant(prim, a, b),
                        tagged(exact),
                        "{prim:?} {x} {y}"
                    );
                    compared += 1;
                }
            }
        }
        // Results at both edges are among them: `LOW + 0`, `HIGH - 0`, `LOW * 1`, `HIGH * 1`.
        assert!(compared > 300, "{compared}");
        // A wrong result fact alone, on tagged operands, still gives an odd word.
        for prim in PRIMS {
            for (x, y) in [(HIGH, HIGH), (LOW, LOW), (HIGH, LOW), (LOW, -1)] {
                let word = unchecked_constant(prim, tagged(x), tagged(y));
                assert_eq!(word & 1, 1, "{prim:?} {x} {y}");
            }
        }
    }

    #[test]
    fn a_wrong_operand_fact_yields_an_odd_word_never_a_handle_2245() {
        // An operand that a wrong fact takes for a tagged word while it is a handle (an even
        // word other than 0, as a promoted `Int` is) or 0. Without the forced tag, `+` and `-`
        // of a handle and a tagged word give an even word: one that could name a live object.
        let handles = [
            2_i64,
            4,
            6,
            0x1000,
            0x7ffe,
            1 << 40,
            i64::MAX - 1,
            i64::MIN,
            0,
        ];
        let tagged_words = [
            tagged(0),
            tagged(1),
            tagged(-1),
            tagged(7),
            tagged((1 << 62) - 1),
        ];
        for prim in PRIMS {
            for h in handles {
                for t in tagged_words {
                    for (a, b) in [(h, t), (t, h), (h, h)] {
                        let word = unchecked_constant(prim, a, b);
                        assert_eq!(word & 1, 1, "{prim:?} {a:#x} {b:#x}: {word:#x}");
                    }
                }
            }
        }
        // The forced tag is what makes it so: `+` of the handle 4 and the tagged 0 is 4 before it.
        assert_eq!(4_i64 - 1 + tagged(0), 4);
        assert_eq!(unchecked_constant(Prim::Add, 4, tagged(0)), 5);
    }

    #[test]
    fn each_type_argument_takes_the_descriptor_of_its_form() {
        // `spec/20 §7`: the descriptor a call passes for each type argument under the
        // constraint of its parameter. A wrong one makes the runtime count a fault (`generic`),
        // which the debug runtime's check reports, so every runnable program of the
        // differential test checks the descriptors its calls pass.
        let cases = [
            (base(Base::Int), Constraint::Integer, Some(Descriptor::Int)),
            (
                base(Base::Fixed(IntType::U64)),
                Constraint::Ord,
                Some(Descriptor::Fixed(IntType::U64)),
            ),
            (
                base(Base::Fixed(IntType::I8)),
                Constraint::Eq,
                Some(Descriptor::Fixed(IntType::I8)),
            ),
            (base(Base::Text), Constraint::Ord, Some(Descriptor::Text)),
            (
                named("app", "Level"),
                Constraint::Ord,
                Some(Descriptor::Sum(5)),
            ),
            (
                named("app", "Box"),
                Constraint::Eq,
                Some(Descriptor::Structural(5)),
            ),
            (
                named(PRELUDE_MODULE, "List"),
                Constraint::Eq,
                Some(Descriptor::Structural(5)),
            ),
            (
                base(Base::Bool),
                Constraint::Eq,
                Some(Descriptor::Structural(5)),
            ),
            (base(Base::Param("T".to_owned())), Constraint::Ord, None),
        ];
        for (ty, constraint, expected) in cases {
            assert_eq!(
                constant_descriptor(&ty, constraint, &mut |_| 5),
                expected,
                "{ty:?}"
            );
        }
        assert_eq!(descriptor_name("T"), "$T");
        assert_eq!(kind(&descriptor_type()), Ok(Kind::Bool));
    }
}
