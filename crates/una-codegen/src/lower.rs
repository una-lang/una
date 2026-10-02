//! The body of one function of Core IR, as Cranelift instructions over words (`spec/16 §4`).

use std::collections::BTreeMap;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{InstBuilder, Value, types};
use cranelift_codegen::isa::TargetFrontendConfig;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::Module;
use una_core::{Atom, Base, Bound, Expr, Fun, HOST_MODULE, Prim, Rhs, Type};

use crate::{CodegenError, Emitter};

/// The integers a tagged word holds: `[-2^62, 2^62)`.
const SMALL: std::ops::RangeInclusive<i64> = -(1 << 62)..=(1 << 62) - 1;

/// What a word holds, as far as the code needs to know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Int,
    Bool,
    Text,
    Unit,
}

fn unsupported(what: impl Into<String>) -> CodegenError {
    CodegenError::Unsupported(what.into())
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
            Base::Named { module, name, .. } if module == "prelude" && name == "List" => {
                Err(unsupported("lists"))
            }
            Base::Param(_) => Err(unsupported("generic functions")),
            _ => Err(unsupported("records and variants")),
        },
        Type::Fn { .. } => Err(unsupported("function values")),
    }
}

/// The binders in scope, with their words and kinds.
type Env = BTreeMap<String, (Value, Kind)>;

struct Lowerer<'e, 'p, 'b> {
    emitter: &'e mut Emitter<'p>,
    builder: FunctionBuilder<'b>,
}

/// Compiles `fun` into `builder`'s function.
pub(crate) fn function(
    emitter: &mut Emitter<'_>,
    mut builder: FunctionBuilder<'_>,
    fun: &Fun,
    config: TargetFrontendConfig,
) -> Result<(), CodegenError> {
    let block = builder.create_block();
    builder.append_block_params_for_function_params(block);
    builder.switch_to_block(block);
    builder.seal_block(block);
    let params = builder.block_params(block).to_vec();
    let mut env = Env::new();
    for ((name, ty), value) in fun.params.iter().zip(params) {
        env.insert(name.clone(), (value, kind(ty)?));
    }
    kind(&fun.result)?;
    let mut lowerer = Lowerer { emitter, builder };
    if let Some(result) = lowerer.expr(&fun.body, &mut env)? {
        lowerer.builder.ins().return_(&[result.0]);
    }
    lowerer.builder.seal_all_blocks();
    lowerer.builder.finalize(config);
    Ok(())
}

impl Lowerer<'_, '_, '_> {
    /// The word of `expr`, or `None` when its block ends without a value (an unreachable
    /// branch).
    fn expr(&mut self, expr: &Expr, env: &mut Env) -> Result<Option<(Value, Kind)>, CodegenError> {
        let mut expr = expr;
        loop {
            match expr {
                Expr::Atom { atom, .. } => return self.atom(atom, env).map(Some),
                Expr::Let {
                    name,
                    ty,
                    value,
                    body,
                    ..
                } => {
                    let kind = kind(ty)?;
                    let bound = match value.as_ref() {
                        Bound::Rhs(rhs) => Some(self.rhs(rhs, kind, env)?),
                        Bound::Expr(inner) => self.expr(inner, &mut env.clone())?,
                    };
                    let Some((value, _)) = bound else {
                        return Ok(None);
                    };
                    env.insert(name.clone(), (value, kind));
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
                    let join = self.builder.create_block();
                    self.builder.append_block_param(join, types::I64);
                    self.builder
                        .ins()
                        .brif(condition, then_block, &[], else_block, &[]);
                    let mut kinds = Vec::new();
                    for (block, branch) in [(then_block, then), (else_block, otherwise)] {
                        self.builder.switch_to_block(block);
                        if let Some((value, kind)) = self.expr(branch, &mut env.clone())? {
                            self.builder.ins().jump(join, &[value.into()]);
                            kinds.push(kind);
                        }
                    }
                    self.builder.switch_to_block(join);
                    let Some(&kind) = kinds.first() else {
                        // Neither branch returns: the join is never reached.
                        self.builder.ins().trap(TRAP);
                        return Ok(None);
                    };
                    let value = self.param(join)?;
                    return Ok(Some((value, kind)));
                }
                Expr::Unreachable { .. } => {
                    // The `exhaustive` obligation proves this branch is never taken.
                    self.builder.ins().trap(TRAP);
                    return Ok(None);
                }
                Expr::Case { .. } => return Err(unsupported("`when` over variants")),
                Expr::Fail { .. } => return Err(unsupported("`fail`")),
                Expr::Handle { .. } => return Err(unsupported("handling clauses")),
                Expr::Attempt { .. } => return Err(unsupported("`attempt`")),
                Expr::With { .. } => return Err(unsupported("`with`")),
            }
        }
    }

    /// The word of a right-hand side whose value has kind `expected`.
    fn rhs(&mut self, rhs: &Rhs, expected: Kind, env: &Env) -> Result<(Value, Kind), CodegenError> {
        match rhs {
            Rhs::Atom(atom) => self.atom(atom, env),
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
                let id = self.emitter.function(function)?;
                let mut values = Vec::new();
                for arg in args {
                    values.push(self.atom(arg, env)?.0);
                }
                // `spec/11 §7`: each call counts toward the maximum depth, which the runtime
                // checks against the call's site.
                let site = i64::try_from(self.emitter.sites.len()).unwrap_or(i64::MAX);
                self.emitter.sites.push(*span);
                let site = self.builder.ins().iconst(types::I64, site);
                self.runtime("una_rt_enter", &[site]);
                let callee = self
                    .emitter
                    .module
                    .declare_func_in_func(id, self.builder.func);
                let call = self.builder.ins().call(callee, &values);
                let result = self.builder.inst_results(call).first().copied();
                self.runtime("una_rt_leave", &[]);
                let result = result.ok_or_else(|| unsupported("this call"))?;
                Ok((result, expected))
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
                let unit = self.builder.ins().iconst(types::I64, 0);
                Ok((unit, Kind::Unit))
            }
            Rhs::Apply { .. } => Err(unsupported("calls of function values")),
            Rhs::Construct { .. } => Err(unsupported("records and variants")),
            Rhs::Field { .. } => Err(unsupported("field access")),
            Rhs::List(_) => Err(unsupported("lists")),
            Rhs::Lambda { .. } => Err(unsupported("lambdas")),
            Rhs::Checked { .. } => Err(unsupported("`checked`")),
            Rhs::Declassify { .. } => Err(unsupported("policies")),
        }
    }

    /// The word of an atom.
    fn atom(&mut self, atom: &Atom, env: &Env) -> Result<(Value, Kind), CodegenError> {
        Ok(match atom {
            Atom::Var(name) => *env
                .get(name.as_str())
                .ok_or_else(|| CodegenError::Internal(format!("`{name}` is not bound")))?,
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
        self.runtime(function, &[address, len])
            .ok_or_else(|| CodegenError::Internal(format!("`{function}` returns nothing")))
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
            .ok_or_else(|| CodegenError::Internal(format!("`{name}` returns nothing")))
    }

    /// The one parameter of a join block.
    fn param(&self, block: cranelift_codegen::ir::Block) -> Result<Value, CodegenError> {
        self.builder
            .block_params(block)
            .first()
            .copied()
            .ok_or_else(|| CodegenError::Internal("a join block lost its parameter".to_owned()))
    }

    /// A Bool word from a comparison's flag.
    fn flag(&mut self, flag: Value) -> Value {
        self.builder.ins().uextend(types::I64, flag)
    }

    fn prim(&mut self, prim: Prim, args: &[(Value, Kind)]) -> Result<(Value, Kind), CodegenError> {
        let first = args.first().copied();
        let second = args.get(1).copied();
        let operand = |i: Option<(Value, Kind)>| {
            i.ok_or_else(|| CodegenError::Internal(format!("`{}` lacks an operand", prim.as_str())))
        };
        match prim {
            Prim::Add | Prim::Sub | Prim::Mul => {
                let (a, b) = (operand(first)?.0, operand(second)?.0);
                Ok((self.arithmetic(prim, a, b)?, Kind::Int))
            }
            Prim::Div => {
                let (a, b) = (operand(first)?.0, operand(second)?.0);
                Ok((self.call("una_rt_int_div", &[a, b])?, Kind::Int))
            }
            Prim::Mod => {
                let (a, b) = (operand(first)?.0, operand(second)?.0);
                Ok((self.call("una_rt_int_mod", &[a, b])?, Kind::Int))
            }
            Prim::Neg => {
                let a = operand(first)?.0;
                Ok((self.call("una_rt_int_neg", &[a])?, Kind::Int))
            }
            Prim::Not => {
                let a = operand(first)?.0;
                Ok((self.builder.ins().bxor_imm_s(a, 1), Kind::Bool))
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
                let value = match kind {
                    Kind::Int => self.compare_ints(cc, a, b)?,
                    Kind::Bool | Kind::Unit => {
                        let flag = self.builder.ins().icmp(cc, a, b);
                        self.flag(flag)
                    }
                    Kind::Text => {
                        let order = if matches!(prim, Prim::Eq | Prim::Ne) {
                            // `text_equal` is 1 when equal; as an order, 0 when equal.
                            let equal = self.call("una_rt_text_equal", &[a, b])?;
                            self.builder.ins().bxor_imm_s(equal, 1)
                        } else {
                            self.call("una_rt_text_compare", &[a, b])?
                        };
                        let flag = self.builder.ins().icmp_imm_s(cc, order, 0);
                        self.flag(flag)
                    }
                };
                Ok((value, Kind::Bool))
            }
            Prim::Len => {
                let (a, kind) = operand(first)?;
                if kind != Kind::Text {
                    return Err(unsupported("the length of a list"));
                }
                Ok((self.call("una_rt_text_len", &[a])?, Kind::Int))
            }
            Prim::Concat => {
                let mut parts = args.iter();
                let Some(&(mut text, _)) = parts.next() else {
                    return Ok((self.literal("una_rt_text_literal", "")?, Kind::Text));
                };
                for &(part, _) in parts {
                    text = self.call("una_rt_text_concat", &[text, part])?;
                }
                Ok((text, Kind::Text))
            }
            Prim::ToText => {
                let (a, kind) = operand(first)?;
                let text = match kind {
                    Kind::Int => self.call("una_rt_int_text", &[a])?,
                    Kind::Bool => self.call("una_rt_bool_text", &[a])?,
                    Kind::Text => a,
                    Kind::Unit => return Err(unsupported("`unit` as text")),
                };
                Ok((text, Kind::Text))
            }
        }
    }

    /// `a op b` over integer words: computed inline when both are tagged and the result fits
    /// 63 bits, else by the runtime, which promotes to arbitrary precision (NB-3).
    fn arithmetic(&mut self, prim: Prim, a: Value, b: Value) -> Result<Value, CodegenError> {
        let fast = self.builder.create_block();
        let slow = self.builder.create_block();
        let done = self.builder.create_block();
        self.builder.append_block_param(done, types::I64);
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
        let done = self.builder.create_block();
        self.builder.append_block_param(done, types::I64);
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

/// The trap of a branch that verification proves unreachable.
const TRAP: cranelift_codegen::ir::TrapCode = cranelift_codegen::ir::TrapCode::unwrap_user(1);
