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
    Atom, Base, Bound, Constraint, Ctor, Decl, Descriptor, Expr, HOST_MODULE, Label, LoopSource,
    MapOp, Name, PRELUDE_MODULE, Path, Prim, Rhs, Source, Type, vars,
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
    /// A handle to a tuple: the state of a loop, or the results of a call with `edit`
    /// parameters (`spec/17 §4`).
    Tuple,
    /// A value of a type parameter: any of the above.
    Any,
}

impl Kind {
    /// Whether the word may be a handle, which reference counting tracks.
    fn counted(self) -> bool {
        !matches!(self, Kind::Bool | Kind::Unit)
    }

    /// Whether the word is always a handle.
    fn handle(self) -> bool {
        matches!(
            self,
            Kind::Text | Kind::List | Kind::Map | Kind::Fn | Kind::Hashed | Kind::Tuple
        )
    }
}

fn unsupported(what: impl Into<String>) -> CodegenError {
    CodegenError::Unsupported(what.into())
}

fn internal(what: impl Into<String>) -> CodegenError {
    CodegenError::Internal(what.into())
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
            // A fixed-width integer is an integer word: its range is proven, so arithmetic on
            // it never leaves the type (`spec/18 §3`).
            Base::Fixed(_) => Ok(Kind::Int),
            Base::Named { module, name, .. } if module == PRELUDE_MODULE && name == "List" => {
                Ok(Kind::List)
            }
            Base::Named { module, name, .. } if module == PRELUDE_MODULE && name == "Map" => {
                Ok(Kind::Map)
            }
            Base::Tuple(_) => Ok(Kind::Tuple),
            Base::Named { .. } => Ok(Kind::Data),
            Base::Param(_) | Base::Unknown => Ok(Kind::Any),
        },
        Type::Fn { .. } => Ok(Kind::Fn),
    }
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
    /// The block that returns the status 2 of a stopped task (`spec/19 §6` rule 4): a task
    /// that is cancelled, or that aborts, stops where it is. Nothing is released, since the
    /// task's heap ends with its thread.
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
    };
    if let Some(result) = lowerer.expr(body.body, &mut env, &mut scope, &catch)? {
        let ok = lowerer.builder.ins().iconst(types::I64, 0);
        lowerer.builder.ins().return_(&[result, ok]);
    }
    lowerer.builder.switch_to_block(failed);
    let failure = lowerer.param(failed)?;
    let one = lowerer.builder.ins().iconst(types::I64, 1);
    lowerer.builder.ins().return_(&[failure, one]);
    lowerer.builder.switch_to_block(stop);
    let zero = lowerer.builder.ins().iconst(types::I64, 0);
    let two = lowerer.builder.ins().iconst(types::I64, 2);
    lowerer.builder.ins().return_(&[zero, two]);
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
                Expr::Together { .. } | Expr::TogetherFor { .. } => {
                    return self.together(expr, env, scope, catch);
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
                    };
                    let mut reached = self.branch(body, env, scope, join, &inner)?;
                    self.builder.switch_to_block(handling);
                    let failure = self.param(handling)?;
                    let mut handler_env = env.clone();
                    handler_env.insert(error.clone(), (failure, unknown()));
                    let mut handler_scope = scope.clone();
                    handler_scope.push((failure, Kind::Any));
                    reached |=
                        self.branch_in(handler, &mut handler_env, &handler_scope, join, catch)?;
                    return self.joined(join, reached);
                }
                Expr::Attempt { body, .. } => return self.attempt(body, env, scope).map(Some),
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
        } = expr
        else {
            return Err(internal("a loop that is not one"));
        };
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
        let stopping = self.call("una_rt_budget", &[site])?;
        self.stop_if(stopping);
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
        // failure in the body releases the state with the body's scope, and the cursor.
        let mut nested = catch.nested(scope);
        if list.is_none() {
            nested.outer.push((bound, Kind::Int));
            nested.outer.push((cursor, Kind::Int));
        }
        let mut owned: Scope = current.iter().copied().zip(kinds.iter().copied()).collect();
        if let Some(tuple) = self.expr(body, &mut inner, &mut owned, &nested)? {
            let mut next = Vec::new();
            for (index, &kind) in kinds.iter().enumerate() {
                let value = self.field(tuple, index)?;
                self.retain(value, kind);
                next.push(value);
            }
            self.release(tuple, Kind::Tuple);
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
        Ok(closure)
    }

    /// `together` and `together for` (`spec/19`), as the value of the expression whose scope is
    /// `scope`: each task is a closure of the bindings before the block, which the runtime runs
    /// on its own thread (`tasks`). The block fails with the failure of the lowest-indexed task
    /// that failed, and stops when its task must.
    fn together(
        &mut self,
        expr: &'p Expr,
        env: &Env,
        scope: &Scope,
        catch: &Catch,
    ) -> Result<Option<Value>, CodegenError> {
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
                        (self.call("una_rt_range", &[low, high, inclusive])?, true)
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
        let status = self.call("una_rt_together_status", &[])?;
        self.stop_if_stopped(status);
        self.check(value, status, scope, catch);
        for &(owned, kind) in scope.iter().rev() {
            self.release(owned, kind);
        }
        Ok(Some(value))
    }

    /// `attempt body`: `Ok` of the body's value, or `Err` of its failure (`spec/11 §4`), as the
    /// value of the expression whose scope is `scope`.
    fn attempt(&mut self, body: &'p Expr, env: &Env, scope: &Scope) -> Result<Value, CodegenError> {
        let succeeded = self.join();
        let failed = self.join();
        let done = self.join();
        let inner = Catch {
            block: failed,
            outer: Scope::new(),
            base: scope.len(),
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
        let err = self.wrap(1, failure)?;
        for &(owned, kind) in scope.iter().rev() {
            self.release(owned, kind);
        }
        self.builder.ins().jump(done, &[err.into()]);
        self.builder.switch_to_block(done);
        self.param(done)
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
    /// lambda (N-42).
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
        span: Span,
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
        // `spec/11 §7`: each call counts toward the maximum depth, which the runtime checks
        // against the call's site.
        let site = self.site(span);
        self.enter(site)?;
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
        self.stop_if_stopped(failed);
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
        self.emitter.ops.push(serde_json::json!({
            "capability": capability,
            "op": op,
            "params": params,
            "args": shown,
            "result": result,
            "error": error,
        }));
        let site = self.builder.ins().iconst(types::I64, site);
        // A cancelled task stops before its next operation (`spec/19 §6` rule 4).
        let stopping = self.call("una_rt_poll", &[])?;
        self.stop_if(stopping);
        // From here the operation is in progress: it completes, and keeps its own outcome,
        // whatever cancellation comes meanwhile (`spec/19 §6` rule 4, `§7` rule 3).
        self.runtime("una_rt_operation_begin", &[]);
        let slot = self.call("una_rt_event", &[site, instance])?;
        for (position, &value) in values.iter().enumerate() {
            let position = i64::try_from(position).map_err(crate::internal)?;
            let position = self.builder.ins().iconst(types::I64, position);
            self.runtime("una_rt_event_arg", &[site, slot, position, value]);
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

        // A host instance has no code: the runtime performs the operation (`spec/14 §3`).
        self.builder.switch_to_block(host);
        if capability_path.module == HOST_MODULE {
            let (function, fails) = match (capability_path.name.as_str(), op.as_str()) {
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
                _ => return Err(unsupported(format!("`{capability}.{op}`"))),
            };
            let sited = matches!(
                (capability_path.name.as_str(), op.as_str()),
                ("Http", _) | ("Env", "get") | ("Files", "read")
            );
            let value = if sited {
                // The call's site, where a header value with a control character aborts with
                // U6004 (N-40), and host text that is not valid UTF-8 with U6005 (S-16, ADR
                // 0086); a task stopped by it stops here.
                let site = self.site(*span);
                let mut operands = vec![site];
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
        self.enter(call_site)?;
        let call = self.builder.ins().call_indirect(signature, code, &operands);
        let results = self.builder.inst_results(call).to_vec();
        self.runtime("una_rt_leave", &[]);
        self.runtime("una_rt_operation_end", &[]);
        let (Some(&value), Some(&failed)) = (results.first(), results.get(1)) else {
            return Err(internal("an operation lost its result"));
        };
        // A task that stops inside the operation, by U6003 alone, leaves its event as it was,
        // `aborted`.
        self.stop_if_stopped(failed);
        self.builder
            .ins()
            .jump(done, &[value.into(), failed.into()]);

        self.builder.switch_to_block(done);
        let results = self.builder.block_params(done).to_vec();
        let (Some(&value), Some(&failed)) = (results.first(), results.get(1)) else {
            return Err(internal("an operation lost its result"));
        };
        self.runtime("una_rt_event_end", &[site, slot, value, failed]);
        // An operation that cannot fail returns 0 as its failure flag, so every operation
        // checks it, as every call does (#267).
        self.check(value, failed, scope, catch);
        Ok(value)
    }

    /// A new instance of `handler` (E-Handler, `spec/11 §4`): the code of each operation of its
    /// capability, in the capability's order, then the values `args` and the instances of its
    /// own capabilities, from those in scope.
    fn install(&mut self, handler: &Path, args: &[Atom], env: &Env) -> Result<Value, CodegenError> {
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
            Rhs::Prim { prim, args, .. } => {
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
                    return self.generic_prim(*prim, &operands);
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
                        // A type that holds type parameters is written through its composite
                        // descriptor (ADR 0068 D6).
                        if !type_params(&ty).is_empty() {
                            let desc = self.descriptor(&ty, Constraint::Eq, env)?;
                            return self.call("una_rt_generic_write", &[desc, value]);
                        }
                        let index =
                            i64::try_from(self.emitter.types.of(&ty)).map_err(crate::internal)?;
                        let index = self.builder.ins().iconst(types::I64, index);
                        return self.call("una_rt_write", &[index, value]);
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
                self.prim(*prim, &operands, width)
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
                            "una_rt_generic_contains",
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
                        let mut operands = vec![desc];
                        operands.extend(values);
                        return self.call(runtime, &operands);
                    }
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
                    // `fold` hands its state to the step as owned (N-42): an initial value at
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
                let borrowed = self.give(function, args, &values, env, scope, catch, rest)?;
                self.call_function(
                    function, borrowed, type_args, &values, *span, env, scope, catch,
                )
            }
            Rhs::Apply {
                callee, args, span, ..
            } => {
                let (callee, _) = self.atom(callee, env)?;
                let mut values = Vec::new();
                for arg in args {
                    values.push(self.atom(arg, env)?.0);
                }
                // A lambda owns every parameter (N-42): each argument moves at its last use,
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
                let borrowed = self.give(
                    via,
                    std::slice::from_ref(arg),
                    &[value],
                    env,
                    scope,
                    catch,
                    rest,
                )?;
                self.call_function(via, borrowed, &[], &[value], *span, env, scope, catch)
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
        self.enter(site)?;
        let call = self.builder.ins().call_indirect(signature, code, &values);
        let results = self.builder.inst_results(call).to_vec();
        self.runtime("una_rt_leave", &[]);
        match (results.first(), results.get(1)) {
            (Some(&value), Some(&failed)) => {
                self.stop_if_stopped(failed);
                Ok((value, failed))
            }
            _ => Err(internal("a closure lost its result")),
        }
    }

    /// Enters the call at `site` (`spec/11 §7`), or stops the task when the runtime says so.
    fn enter(&mut self, site: Value) -> Result<(), CodegenError> {
        let stopping = self.call("una_rt_enter", &[site])?;
        self.stop_if(stopping);
        Ok(())
    }

    /// `set` of the prelude, by `function` (`una_rt_set` or `una_rt_set_owned`), at the call's
    /// site: an index outside the list aborts there with U8001, as the interpreter's fault does
    /// (#480, `spec/17 §7.3`), and a task stopped by it stops here.
    fn set(
        &mut self,
        function: &'static str,
        args: &[Value],
        span: Span,
    ) -> Result<Value, CodegenError> {
        let site = self.site(span);
        let mut operands = vec![site];
        operands.extend_from_slice(args);
        let value = self.call(function, &operands)?;
        let stopped = self.call("una_rt_stopped", &[])?;
        self.stop_if(stopped);
        Ok(value)
    }

    /// Stops the task when `stopping` is not 0.
    fn stop_if(&mut self, stopping: Value) {
        let next = self.builder.create_block();
        self.builder.ins().brif(stopping, self.stop, &[], next, &[]);
        self.builder.switch_to_block(next);
    }

    /// Stops the task when the code it called returned the status 2 of a stopped task.
    fn stop_if_stopped(&mut self, status: Value) {
        let stopped = self.builder.ins().icmp_imm_s(IntCC::Equal, status, 2);
        let stopped = self.flag(stopped);
        self.stop_if(stopped);
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
            "split" => "una_rt_split",
            "join" => "una_rt_join",
            "ends_with" => "una_rt_ends_with",
            "contains_text" => "una_rt_contains_text",
            "find" => "una_rt_find",
            "replace" => "una_rt_replace",
            "slice" => "una_rt_slice",
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
            "contains" => "una_rt_contains",
            "set" => return self.set("una_rt_set", args, span),
            "sum" => "una_rt_sum",
            "or_else" => "una_rt_or_else",
            "empty_map" => "una_rt_map_empty",
            // JSON (`spec/23`): `parse_json` reports its failure as a host operation does.
            // Both check the memory budget at this call's site as they go: over it, the run
            // aborts with U6003 there, or the task stops (N-41).
            "parse_json" => {
                let site = self.site(span);
                let value = self.call("una_rt_parse_json", &[arg(0)?, site])?;
                let stopping = self.call("una_rt_json_stopped", &[])?;
                self.stop_if(stopping);
                let failed = self.call("una_rt_host_failed", &[])?;
                self.check(value, failed, scope, catch);
                return Ok(value);
            }
            "to_json" => {
                let site = self.site(span);
                let value = self.call("una_rt_to_json", &[arg(0)?, site])?;
                let stopping = self.call("una_rt_json_stopped", &[])?;
                self.stop_if(stopping);
                return Ok(value);
            }
            "quote_json" => "una_rt_quote_json",
            "is_json_number" => "una_rt_is_json_number",
            other => return Err(unsupported(format!("`{other}` of the prelude"))),
        };
        self.call(function, args)
    }

    /// `map`, `filter`, `first_where` and `fold`: a loop over the list's items that applies the
    /// function to each, in order.
    /// In edition 0.2 the function may fail (ADR 0062): its failure stops the loop, releases what
    /// the loop made, and goes to `catch`, after the effects already performed.
    /// Every call of the function gives it its arguments as owned (N-42): an item is retained
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
    fn generic_prim(&mut self, prim: Prim, operands: &[Value]) -> Result<Value, CodegenError> {
        let cc = match prim {
            Prim::Convert(_) => return self.call("una_rt_generic_to_int", operands),
            // Interpolation, by the type that instantiates the parameter (ADR 0068).
            Prim::ToText => return self.call("una_rt_generic_write", operands),
            Prim::Eq | Prim::Ne => {
                let equal = self.call("una_rt_generic_equal", operands)?;
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
    ) -> Result<Value, CodegenError> {
        let first = args.first().copied();
        let second = args.get(1).copied();
        let operand = |i: Option<(Value, Kind)>| {
            i.ok_or_else(|| internal(format!("`{}` lacks an operand", prim.as_str())))
        };
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
                        // `equal` is 1 when equal; as an order, 0 when equal.
                        let equal = self.call("una_rt_equal", &[a, b])?;
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
                    Kind::Int => self.call("una_rt_int_text", &[a]),
                    Kind::Bool => self.call("una_rt_bool_text", &[a]),
                    Kind::Text => {
                        self.retain(a, Kind::Text);
                        Ok(a)
                    }
                    _ => Err(unsupported("this value as text")),
                }
            }
        }
    }

    /// `a op b` over tagged words whose result is proven to fit one: plain machine arithmetic.
    fn unchecked(&mut self, prim: Prim, a: Value, b: Value) -> Value {
        // With `a = 2x + 1` and `b = 2y + 1`, as in `arithmetic`.
        match prim {
            Prim::Add => {
                let untagged = self.builder.ins().iadd_imm_s(a, -1);
                self.builder.ins().iadd(untagged, b)
            }
            Prim::Sub => {
                let difference = self.builder.ins().isub(a, b);
                self.builder.ins().iadd_imm_s(difference, 1)
            }
            _ => {
                let x = self.builder.ins().sshr_imm_s(a, 1);
                let twice_y = self.builder.ins().iadd_imm_s(b, -1);
                let product = self.builder.ins().imul(x, twice_y);
                self.builder.ins().iadd_imm_s(product, 1)
            }
        }
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
        Expr::Loop { source, body, .. } => {
            if let LoopSource::List(list) = source.as_ref() {
                add(list, out);
            }
            borrowed_from(body, out);
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
        Expr::Attempt { body, .. } | Expr::With { body, .. } => borrowed_from(body, out),
        Expr::Together { .. } | Expr::TogetherFor { .. } => {
            // A task's captures are never moved: every name the block reads stays pinned
            // (#162).
            vars(expr, out);
        }
        Expr::Atom { .. } | Expr::Fail { .. } | Expr::Unreachable { .. } => {}
    }
}

/// The trap of a branch that verification proves unreachable.
const TRAP: cranelift_codegen::ir::TrapCode = cranelift_codegen::ir::TrapCode::unwrap_user(1);

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
