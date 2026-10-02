//! The body of one function of Core IR, as Cranelift instructions over words (`spec/16 §4`).
//!
//! **Reference counting** (NB-2). A word that a binder holds is either owned, and released at
//! the end of the binder's scope, or borrowed from a value that outlives the binder:
//! - a `let` owns what its right-hand side makes, and every right-hand side returns an owned
//!   word, retaining a borrowed one;
//! - parameters, captured values and the fields that a `case` arm binds are borrowed from the
//!   caller, the closure and the scrutinee;
//! - literals are borrowed from the runtime, which keeps them for the whole run;
//! - calls borrow their arguments and return an owned word.
//!
//! Each path through an expression ends with its result: it retains the result, then releases
//! every word its scope owns.

use std::collections::{BTreeMap, BTreeSet};

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{AbiParam, Block, InstBuilder, Value, types};
use cranelift_codegen::isa::TargetFrontendConfig;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use una_core::{
    Atom, Base, Bound, Ctor, Decl, Expr, HOST_MODULE, Label, Name, PRELUDE_MODULE, Prim, Rhs,
    Source, Type,
};
use una_syntax::Span;

use crate::{CodegenError, Emitter};

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
    /// A handle to a closure.
    Fn,
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
        matches!(self, Kind::Text | Kind::List | Kind::Fn)
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
            Base::Hashed => Err(unsupported("`Hashed` values")),
            Base::Fixed(_) => Err(unsupported("fixed-width integers")),
            Base::Named { module, name, .. } if module == PRELUDE_MODULE && name == "List" => {
                Ok(Kind::List)
            }
            Base::Named { .. } => Ok(Kind::Data),
            Base::Param(_) | Base::Unknown => Ok(Kind::Any),
        },
        Type::Fn { .. } => Ok(Kind::Fn),
    }
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

/// The binders in scope, with their words and types.
type Env = BTreeMap<String, (Value, Type)>;

/// The words a scope owns, released at the end of each of its paths.
type Scope = Vec<(Value, Kind)>;

/// A body to compile: a function or predicate of the package, or a lambda.
pub(crate) struct Body<'p> {
    /// Whether it is the code of a closure, whose first parameter is the closure itself.
    pub(crate) closure: bool,
    /// The values a closure captures, in the order of its environment.
    pub(crate) captures: Vec<(Name, Type)>,
    /// The parameters.
    pub(crate) params: &'p [(Name, Type)],
    /// The result type, when it is declared.
    pub(crate) result: Option<&'p Type>,
    /// The body.
    pub(crate) body: &'p Expr,
}

struct Lowerer<'e, 'p, 'b> {
    emitter: &'e mut Emitter<'p>,
    builder: FunctionBuilder<'b>,
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
    let mut lowerer = Lowerer { emitter, builder };
    let mut env = Env::new();
    if body.closure {
        let this = params
            .next()
            .ok_or_else(|| internal("a closure lost its parameter"))?;
        for (index, (name, ty)) in body.captures.iter().enumerate() {
            kind(ty)?;
            let value = lowerer.field(this, index)?;
            env.insert(name.clone(), (value, ty.clone()));
        }
    }
    for ((name, ty), value) in body.params.iter().zip(params) {
        kind(ty)?;
        env.insert(name.clone(), (value, ty.clone()));
    }
    if let Some(result) = body.result {
        kind(result)?;
    }
    if let Some(result) = lowerer.expr(body.body, &mut env, &mut Scope::new())? {
        lowerer.builder.ins().return_(&[result]);
    }
    lowerer.builder.seal_all_blocks();
    lowerer.builder.finalize(config);
    Ok(())
}

impl<'p> Lowerer<'_, 'p, '_> {
    /// The owned word of `expr`, or `None` when its block ends without a value (an unreachable
    /// branch). `scope` holds the words to release at the end of each path.
    fn expr(
        &mut self,
        expr: &'p Expr,
        env: &mut Env,
        scope: &mut Scope,
    ) -> Result<Option<Value>, CodegenError> {
        let mut expr = expr;
        loop {
            match expr {
                Expr::Atom { atom, .. } => {
                    let (value, kind) = self.atom(atom, env)?;
                    self.retain(value, kind);
                    for &(owned, kind) in scope.iter().rev() {
                        self.release(owned, kind);
                    }
                    return Ok(Some(value));
                }
                Expr::Let {
                    name,
                    ty,
                    value,
                    body,
                    ..
                } => {
                    let kind = kind(ty)?;
                    let bound = match value.as_ref() {
                        Bound::Rhs(rhs) => self.rhs(rhs, ty, env)?,
                        Bound::Expr(inner) => {
                            match self.expr(inner, &mut env.clone(), &mut Scope::new())? {
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
                        reached |= self.branch(branch, env, scope, join)?;
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
                                let field_ty = self.field_type(&ty, &arm.ctor, position);
                                kind(&field_ty)?;
                                let field = self.field(value, position)?;
                                inner.insert(name.clone(), (field, field_ty));
                            }
                        }
                        reached |= self.branch_in(&arm.body, &mut inner, scope, join)?;
                        self.builder.switch_to_block(next);
                    }
                    match otherwise {
                        Some(otherwise) => reached |= self.branch(otherwise, env, scope, join)?,
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
                Expr::Fail { .. } => return Err(unsupported("`fail`")),
                Expr::Handle { .. } => return Err(unsupported("handling clauses")),
                Expr::Attempt { .. } => return Err(unsupported("`attempt`")),
                Expr::With { .. } => return Err(unsupported("`with`")),
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
    ) -> Result<bool, CodegenError> {
        self.branch_in(branch, &mut env.clone(), scope, join)
    }

    fn branch_in(
        &mut self,
        branch: &'p Expr,
        env: &mut Env,
        scope: &Scope,
        join: Block,
    ) -> Result<bool, CodegenError> {
        match self.expr(branch, env, &mut scope.clone())? {
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

    /// The owned word of a right-hand side bound to a binder of type `ty`.
    fn rhs(&mut self, rhs: &'p Rhs, ty: &Type, env: &Env) -> Result<Value, CodegenError> {
        match rhs {
            Rhs::Atom(atom) => {
                let (value, kind) = self.atom(atom, env)?;
                self.retain(value, kind);
                Ok(value)
            }
            Rhs::Prim { prim, args, .. } => {
                let mut operands = Vec::new();
                for arg in args {
                    operands.push(self.atom(arg, env)?);
                }
                self.prim(*prim, &operands)
            }
            Rhs::Call {
                function,
                args,
                span,
                ..
            } => {
                let mut values = Vec::new();
                for arg in args {
                    values.push(self.atom(arg, env)?.0);
                }
                if function.module == PRELUDE_MODULE {
                    return self.prelude(&function.name, &values, *span);
                }
                let id = self.emitter.function(function)?;
                // `spec/11 §7`: each call counts toward the maximum depth, which the runtime
                // checks against the call's site.
                let site = self.site(*span);
                self.runtime("una_rt_enter", &[site]);
                let callee = self
                    .emitter
                    .module
                    .declare_func_in_func(id, self.builder.func);
                let call = self.builder.ins().call(callee, &values);
                let result = self.builder.inst_results(call).first().copied();
                self.runtime("una_rt_leave", &[]);
                result.ok_or_else(|| internal("a call lost its result"))
            }
            Rhs::Apply {
                callee, args, span, ..
            } => {
                let (callee, _) = self.atom(callee, env)?;
                let mut values = Vec::new();
                for arg in args {
                    values.push(self.atom(arg, env)?.0);
                }
                let site = self.site(*span);
                self.apply(callee, &values, site)
            }
            Rhs::Op {
                capability_path,
                op,
                args,
                labels,
                capability,
                ..
            } => {
                if capability_path.module != HOST_MODULE
                    || capability_path.name != "Console"
                    || op != "print"
                {
                    return Err(unsupported(format!("`{capability}.{op}`")));
                }
                if labels.iter().any(|label| !label.is_public()) {
                    return Err(unsupported("labeled arguments of host operations"));
                }
                let [line] = args.as_slice() else {
                    return Err(unsupported("this call of `Console.print`"));
                };
                let (line, _) = self.atom(line, env)?;
                self.runtime("una_rt_console_print", &[line]);
                Ok(self.builder.ins().iconst(types::I64, 0))
            }
            Rhs::Construct { ctor, args, .. } => {
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
            Rhs::Lambda { params, body, .. } => {
                let mut names = BTreeSet::new();
                vars(body, &mut names);
                let captures: Vec<(Name, Type)> = names
                    .into_iter()
                    .filter(|name| !params.iter().any(|(p, _)| p == name))
                    .filter_map(|name| {
                        let ty = env.get(&name)?.1.clone();
                        Some((name, ty))
                    })
                    .collect();
                let values: Vec<Value> = captures
                    .iter()
                    .filter_map(|(name, _)| env.get(name).map(|(value, _)| *value))
                    .collect();
                let id = self.emitter.lambda(params, body, captures)?;
                let code = self
                    .emitter
                    .module
                    .declare_func_in_func(id, self.builder.func);
                let address = self.builder.ins().func_addr(types::I64, code);
                let closure = self.call("una_rt_closure", &[address])?;
                for value in values {
                    self.runtime("una_rt_push", &[closure, value]);
                }
                Ok(closure)
            }
            Rhs::Checked { .. } => Err(unsupported("`checked`")),
            Rhs::Declassify { .. } => Err(unsupported("policies")),
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
            let arg = match (ctor.kind.name.as_str(), ctor.name.as_str()) {
                ("Option", "Some") | ("Result", "Ok") => args.first(),
                ("Result", "Err") => args.get(1),
                _ => None,
            };
            return arg.cloned().unwrap_or_else(unknown);
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
    /// maximum depth at `site`.
    fn apply(&mut self, callee: Value, args: &[Value], site: Value) -> Result<Value, CodegenError> {
        let code = self.call("una_rt_closure_code", &[callee])?;
        let mut signature = self.emitter.module.make_signature();
        for _ in 0..=args.len() {
            signature.params.push(AbiParam::new(types::I64));
        }
        signature.returns.push(AbiParam::new(types::I64));
        let signature = self.builder.import_signature(signature);
        let mut values = vec![callee];
        values.extend_from_slice(args);
        self.runtime("una_rt_enter", &[site]);
        let call = self.builder.ins().call_indirect(signature, code, &values);
        let result = self.builder.inst_results(call).first().copied();
        self.runtime("una_rt_leave", &[]);
        result.ok_or_else(|| internal("a closure lost its result"))
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
    fn prelude(&mut self, name: &str, args: &[Value], span: Span) -> Result<Value, CodegenError> {
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
                return self.higher_order(name, args, site);
            }
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
            "sum" => "una_rt_sum",
            "or_else" => "una_rt_or_else",
            other => return Err(unsupported(format!("`{other}` of the prelude"))),
        };
        self.call(function, args)
    }

    /// `map`, `filter`, `first_where` and `fold`: a loop over the list's items that applies the
    /// function to each, in order.
    fn higher_order(
        &mut self,
        name: &str,
        args: &[Value],
        site: Value,
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
                self.retain(initial, Kind::Any);
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
        let next_index = self.builder.ins().iadd_imm_s(index, 1);
        match name {
            "map" => {
                let mapped = self.apply(function, &[item], site)?;
                self.runtime("una_rt_push", &[carried, mapped]);
                self.release(mapped, Kind::Any);
                self.builder
                    .ins()
                    .jump(head, &[next_index.into(), carried.into()]);
            }
            "filter" | "first_where" => {
                let keep = self.apply(function, &[item], site)?;
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
                let step = self.apply(function, &[carried, item], site)?;
                self.release(carried, Kind::Any);
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

    fn prim(&mut self, prim: Prim, args: &[(Value, Kind)]) -> Result<Value, CodegenError> {
        let first = args.first().copied();
        let second = args.get(1).copied();
        let operand = |i: Option<(Value, Kind)>| {
            i.ok_or_else(|| internal(format!("`{}` lacks an operand", prim.as_str())))
        };
        match prim {
            Prim::Convert(_) | Prim::Fixed(..) => Err(unsupported("fixed-width integers")),
            Prim::Add | Prim::Sub | Prim::Mul => {
                let (a, b) = (operand(first)?.0, operand(second)?.0);
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

/// The word of `None`, the variant of `Option` at position 1, without fields.
const NONE: i64 = (1 << 1) | 1;

/// Every binder that `expr` reads, in it and in the lambdas it makes.
fn vars(expr: &Expr, out: &mut BTreeSet<String>) {
    let atoms = |list: &[Atom], out: &mut BTreeSet<String>| {
        for atom in list {
            if let Atom::Var(name) = atom {
                out.insert(name.clone());
            }
        }
    };
    match expr {
        Expr::Atom { atom, .. } => atoms(std::slice::from_ref(atom), out),
        Expr::Let { value, body, .. } => {
            match value.as_ref() {
                Bound::Rhs(rhs) => rhs_vars(rhs, out),
                Bound::Expr(inner) => vars(inner, out),
            }
            vars(body, out);
        }
        Expr::If {
            cond,
            then,
            otherwise,
        } => {
            atoms(std::slice::from_ref(cond), out);
            vars(then, out);
            vars(otherwise, out);
        }
        Expr::Case {
            scrutinee,
            arms,
            otherwise,
        } => {
            atoms(std::slice::from_ref(scrutinee), out);
            for arm in arms {
                vars(&arm.body, out);
            }
            if let Some(otherwise) = otherwise {
                vars(otherwise, out);
            }
        }
        Expr::Fail { args, .. } => atoms(args, out),
        Expr::Handle { body, handler, .. } => {
            vars(body, out);
            vars(handler, out);
        }
        Expr::Attempt { body, .. } => vars(body, out),
        Expr::With { source, body, .. } => {
            if let Source::Handler { args, .. } = source {
                atoms(args, out);
            }
            vars(body, out);
        }
        Expr::Unreachable { .. } => {}
    }
}

fn rhs_vars(rhs: &Rhs, out: &mut BTreeSet<String>) {
    let mut add = |atom: &Atom| {
        if let Atom::Var(name) = atom {
            out.insert(name.clone());
        }
    };
    match rhs {
        Rhs::Atom(atom) => add(atom),
        Rhs::Prim { args, .. }
        | Rhs::Call { args, .. }
        | Rhs::Op { args, .. }
        | Rhs::Construct { args, .. }
        | Rhs::List(args) => args.iter().for_each(add),
        Rhs::Apply { callee, args, .. } => {
            add(callee);
            args.iter().for_each(add);
        }
        Rhs::Field { value, .. } => add(value),
        Rhs::Declassify { arg, .. } => add(arg),
        Rhs::Checked {
            value,
            args,
            predicate,
            ..
        } => {
            add(value);
            args.iter().for_each(add);
            vars(predicate, out);
        }
        Rhs::Lambda { body, .. } => vars(body, out),
    }
}

/// The trap of a branch that verification proves unreachable.
const TRAP: cranelift_codegen::ir::TrapCode = cranelift_codegen::ir::TrapCode::unwrap_user(1);
