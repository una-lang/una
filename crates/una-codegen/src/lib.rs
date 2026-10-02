//! Native code generation (`spec/16`, ADR 0055): Core IR of a checked and verified package to an
//! object file for the host target, through Cranelift. `una build` links the object with the
//! runtime library `una-rt` through the system `cc`.
//!
//! The backend is delivered in slices (#26). This slice compiles `main` of the entry module when
//! its body prints text through the host `Console`: enough for `examples/hello`. Any other
//! construct is [`CodegenError::Unsupported`], with the construct named, so that `una build`
//! reports what the native backend cannot compile yet instead of building something else.
//!
//! The object has two functions. `una_main` is `main` of Una; it returns 0 when `main`
//! returns. `main`, the C entry point, calls `una_rt_start` of the runtime with the process
//! arguments, the program's description that `una build` embeds (`spec/16 §2`) and
//! `una_main`. This crate emits machine code but never runs it: there is no JIT, and no
//! `unsafe`.
#![deny(missing_docs)]

use std::collections::BTreeMap;
use std::fmt;

use cranelift_codegen::ir::{AbiParam, InstBuilder, UserFuncName, Value, types};
use cranelift_codegen::isa::{self, OwnedTargetIsa};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::{Context, ir};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};
use target_lexicon::Triple;
use una_core::{Atom, Bound, Decl, Expr, Fun, HOST_MODULE, Program, Rhs};

/// Why the backend cannot compile a program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodegenError {
    /// The program uses a construct that the backend does not compile yet (#26).
    Unsupported(String),
    /// The entry module has no `main`.
    NoMain,
    /// Cranelift rejected what the backend produced: a bug of the backend (U8001).
    Internal(String),
}

impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodegenError::Unsupported(what) => {
                write!(f, "the native backend does not compile {what} yet")
            }
            CodegenError::NoMain => write!(f, "the entry module has no `main`"),
            CodegenError::Internal(message) => {
                write!(f, "native code generation failed: {message}")
            }
        }
    }
}

/// The host target, as `rustc` names it: `aarch64-apple-darwin` or
/// `x86_64-unknown-linux-gnu` (`spec/16 §1`).
pub fn host_target() -> String {
    Triple::host().to_string()
}

/// The object file of the program: `main` of module `entry` compiled for the host target, and
/// `meta`, the description of the program that the runtime reads at start (`spec/16 §2`).
pub fn compile(program: &Program, entry: &str, meta: &str) -> Result<Vec<u8>, CodegenError> {
    let main = program
        .modules
        .iter()
        .filter(|module| module.path == entry)
        .flat_map(|module| &module.decls)
        .find_map(|decl| match decl {
            Decl::Fun(fun) if fun.name == "main" => Some(fun),
            _ => None,
        })
        .ok_or(CodegenError::NoMain)?;
    let mut emitter = Emitter::new(isa()?)?;
    let body = emitter.lower_main(main)?;
    emitter.entry(body, meta)?;
    emitter
        .module
        .finish()
        .emit()
        .map_err(|error| CodegenError::Internal(error.to_string()))
}

fn internal(error: impl fmt::Display) -> CodegenError {
    CodegenError::Internal(error.to_string())
}

/// The host's instruction set, position independent, as executables on both targets need.
fn isa() -> Result<OwnedTargetIsa, CodegenError> {
    let mut flags = settings::builder();
    flags.set("is_pic", "true").map_err(internal)?;
    flags.set("opt_level", "speed").map_err(internal)?;
    isa::lookup(Triple::host())
        .map_err(internal)?
        .finish(settings::Flags::new(flags))
        .map_err(internal)
}

/// What `main` does, in order: the host operations it performs.
enum Step {
    /// `Console.print` of a text known at compile time.
    Print(String),
}

struct Emitter {
    module: ObjectModule,
    texts: BTreeMap<String, DataId>,
}

impl Emitter {
    fn new(isa: OwnedTargetIsa) -> Result<Emitter, CodegenError> {
        let builder = ObjectBuilder::new(isa, "una", default_libcall_names()).map_err(internal)?;
        Ok(Emitter {
            module: ObjectModule::new(builder),
            texts: BTreeMap::new(),
        })
    }

    /// The steps of `main`'s body. This slice compiles a sequence of `let`s whose values are
    /// text literals or `Console.print` of a text, ending in `unit`.
    fn lower_main(&mut self, main: &Fun) -> Result<Vec<Step>, CodegenError> {
        let mut texts: BTreeMap<&str, &str> = BTreeMap::new();
        let mut steps = Vec::new();
        let mut expr = &main.body;
        loop {
            match expr {
                Expr::Atom {
                    atom: Atom::Unit, ..
                } => return Ok(steps),
                Expr::Atom {
                    atom: Atom::Var(name),
                    ..
                } if !texts.contains_key(name.as_str()) => {
                    return Ok(steps);
                }
                Expr::Let {
                    name, value, body, ..
                } => {
                    match value.as_ref() {
                        Bound::Rhs(Rhs::Atom(Atom::Text(text))) => {
                            texts.insert(name, text);
                        }
                        Bound::Rhs(Rhs::Op {
                            capability_path,
                            op,
                            args,
                            ..
                        }) if capability_path.module == HOST_MODULE
                            && capability_path.name == "Console"
                            && op == "print" =>
                        {
                            let text = match args.as_slice() {
                                [Atom::Text(text)] => text.as_str(),
                                [Atom::Var(var)] => {
                                    texts.get(var.as_str()).copied().ok_or_else(|| {
                                        CodegenError::Unsupported("a computed `Text`".to_owned())
                                    })?
                                }
                                _ => {
                                    return Err(CodegenError::Unsupported(
                                        "this argument of `Console.print`".to_owned(),
                                    ));
                                }
                            };
                            self.text(text)?;
                            steps.push(Step::Print(text.to_owned()));
                        }
                        Bound::Rhs(rhs) => {
                            return Err(CodegenError::Unsupported(describe_rhs(rhs)));
                        }
                        Bound::Expr(inner) => {
                            return Err(CodegenError::Unsupported(describe_expr(inner)));
                        }
                    }
                    expr = body;
                }
                other => return Err(CodegenError::Unsupported(describe_expr(other))),
            }
        }
    }

    /// The read-only data of a text literal, emitted once per text.
    fn text(&mut self, text: &str) -> Result<DataId, CodegenError> {
        if let Some(id) = self.texts.get(text) {
            return Ok(*id);
        }
        let name = format!("una_text_{}", self.texts.len());
        let id = self
            .module
            .declare_data(&name, Linkage::Local, false, false)
            .map_err(internal)?;
        let mut data = DataDescription::new();
        data.define(text.as_bytes().to_vec().into_boxed_slice());
        self.module.define_data(id, &data).map_err(internal)?;
        self.texts.insert(text.to_owned(), id);
        Ok(id)
    }

    /// `una_main`, which performs the steps and returns 0, and the C `main`, which hands the
    /// process to the runtime.
    fn entry(&mut self, steps: Vec<Step>, meta: &str) -> Result<(), CodegenError> {
        let pointer = self.module.target_config().pointer_type();
        let call_conv = self.module.target_config().default_call_conv;

        let mut print = self.module.make_signature();
        print.params.push(AbiParam::new(pointer));
        print.params.push(AbiParam::new(pointer));
        let print = self
            .module
            .declare_function("una_rt_console_print", Linkage::Import, &print)
            .map_err(internal)?;

        let mut una_main = self.module.make_signature();
        una_main.returns.push(AbiParam::new(types::I64));
        let una_main_id = self
            .module
            .declare_function("una_main", Linkage::Local, &una_main)
            .map_err(internal)?;
        let mut context = self.module.make_context();
        context.func.signature = una_main;
        context.func.name = UserFuncName::user(0, una_main_id.as_u32());
        {
            let mut builder_context = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
            let block = builder.create_block();
            builder.switch_to_block(block);
            builder.seal_block(block);
            let print_ref = self.module.declare_func_in_func(print, builder.func);
            for step in &steps {
                match step {
                    Step::Print(text) => {
                        let (address, len) = self.text_value(&mut builder, text, pointer)?;
                        builder.ins().call(print_ref, &[address, len]);
                    }
                }
            }
            let zero = builder.ins().iconst(types::I64, 0);
            builder.ins().return_(&[zero]);
            builder.finalize(self.module.target_config());
        }
        self.define(una_main_id, &mut context)?;

        let meta_id = self
            .module
            .declare_data("una_meta", Linkage::Local, false, false)
            .map_err(internal)?;
        let mut data = DataDescription::new();
        data.define(meta.as_bytes().to_vec().into_boxed_slice());
        self.module.define_data(meta_id, &data).map_err(internal)?;

        let mut start = self.module.make_signature();
        start.params.push(AbiParam::new(types::I32));
        start.params.push(AbiParam::new(pointer));
        start.params.push(AbiParam::new(pointer));
        start.params.push(AbiParam::new(pointer));
        start.params.push(AbiParam::new(pointer));
        start.returns.push(AbiParam::new(types::I32));
        let start = self
            .module
            .declare_function("una_rt_start", Linkage::Import, &start)
            .map_err(internal)?;

        let mut c_main = ir::Signature::new(call_conv);
        c_main.params.push(AbiParam::new(types::I32));
        c_main.params.push(AbiParam::new(pointer));
        c_main.returns.push(AbiParam::new(types::I32));
        let c_main_id = self
            .module
            .declare_function("main", Linkage::Export, &c_main)
            .map_err(internal)?;
        let mut context = self.module.make_context();
        context.func.signature = c_main;
        context.func.name = UserFuncName::user(0, c_main_id.as_u32());
        {
            let mut builder_context = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
            let block = builder.create_block();
            builder.append_block_params_for_function_params(block);
            builder.switch_to_block(block);
            builder.seal_block(block);
            let params = builder.block_params(block).to_vec();
            let (Some(&argc), Some(&argv)) = (params.first(), params.get(1)) else {
                return Err(internal("`main` lost its parameters"));
            };
            let meta_value = self.module.declare_data_in_func(meta_id, builder.func);
            let meta_address = builder.ins().symbol_value(pointer, meta_value);
            let meta_len = builder
                .ins()
                .iconst(pointer, i64::try_from(meta.len()).map_err(internal)?);
            let una_main_ref = self.module.declare_func_in_func(una_main_id, builder.func);
            let una_main_address = builder.ins().func_addr(pointer, una_main_ref);
            let start_ref = self.module.declare_func_in_func(start, builder.func);
            let call = builder.ins().call(
                start_ref,
                &[argc, argv, meta_address, meta_len, una_main_address],
            );
            let status = builder.inst_results(call).to_vec();
            builder.ins().return_(&status);
            builder.finalize(self.module.target_config());
        }
        self.define(c_main_id, &mut context)
    }

    /// The address and length of a text's data, inside `builder`'s function.
    fn text_value(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        text: &str,
        pointer: ir::Type,
    ) -> Result<(Value, Value), CodegenError> {
        let id = self.text(text)?;
        let global = self.module.declare_data_in_func(id, builder.func);
        let address = builder.ins().symbol_value(pointer, global);
        let len = builder
            .ins()
            .iconst(pointer, i64::try_from(text.len()).map_err(internal)?);
        Ok((address, len))
    }

    fn define(&mut self, id: FuncId, context: &mut Context) -> Result<(), CodegenError> {
        self.module
            .define_function(id, context)
            .map_err(|error| internal(format!("{error:?}")))?;
        self.module.clear_context(context);
        Ok(())
    }
}

/// How an unsupported right-hand side reads in the message.
fn describe_rhs(rhs: &Rhs) -> String {
    match rhs {
        Rhs::Atom(Atom::Int(_)) => "`Int` values".to_owned(),
        Rhs::Atom(Atom::Bool(_)) => "`Bool` values".to_owned(),
        Rhs::Atom(_) => "this value".to_owned(),
        Rhs::Prim { .. } => "arithmetic, comparisons and other primitive operations".to_owned(),
        Rhs::Call { .. } => "calls of functions".to_owned(),
        Rhs::Apply { .. } => "calls of function values".to_owned(),
        Rhs::Op { capability, op, .. } => format!("`{capability}.{op}`"),
        Rhs::Construct { .. } => "records and variants".to_owned(),
        Rhs::Field { .. } => "field access".to_owned(),
        Rhs::List(_) => "lists".to_owned(),
        Rhs::Lambda { .. } => "lambdas".to_owned(),
        Rhs::Checked { .. } => "`checked`".to_owned(),
        Rhs::Declassify { .. } => "policies".to_owned(),
    }
}

/// How an unsupported expression reads in the message.
fn describe_expr(expr: &Expr) -> String {
    match expr {
        Expr::Atom { .. } => "this result of `main`".to_owned(),
        Expr::Let { .. } => "this `let`".to_owned(),
        Expr::If { .. } => "`if`".to_owned(),
        Expr::Case { .. } => "`when`".to_owned(),
        Expr::Fail { .. } => "`fail`".to_owned(),
        Expr::Handle { .. } => "handling clauses".to_owned(),
        Expr::Attempt { .. } => "`attempt`".to_owned(),
        Expr::With { .. } => "`with`".to_owned(),
        Expr::Unreachable { .. } => "a `when` without `else`".to_owned(),
    }
}
