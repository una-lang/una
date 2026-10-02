//! Native code generation (`spec/16`, ADR 0055): Core IR of a checked and verified package to an
//! object file for the host target, through Cranelift. `una build` links the object with the
//! runtime library `una-rt` through the system `cc`.
//!
//! The backend is delivered in slices (#26). It compiles `main` of the entry module and every
//! function it reaches, when they use what the backend compiles so far:
//! - integers (tagged words, promoted to arbitrary precision by the runtime, NB-3), `Bool`,
//!   `Unit`, and `Text` literals, interpolation and comparison;
//! - arithmetic with Euclidean division, comparisons, `not`, `if`, and the `when` over
//!   literals and ranges that Core writes as `if`;
//! - calls of the package's functions and predicates, recursion included, with the call depth
//!   of `spec/11 §7`;
//! - records and variants, `when` over them, `List`, `Option` and `Result`, and the functions of
//!   the prelude;
//! - generic functions, compiled once, since every value is a word;
//! - lambdas and closures, and declared functions used as values;
//! - failures, handling clauses, `attempt`, `checked` and policies: every compiled body returns
//!   its value and whether it failed, and nothing unwinds (`spec/16 §4` rule 3); a failure of
//!   `main` is U6001, which the runtime shows from the types `una build` describes;
//! - `Console.print`.
//!
//! Heap values are reference counted (NB-2): `lower` describes which words the code owns and
//! when it releases them. A debug build of the runtime reports a value the code leaks or uses
//! after releasing it, which the differential test turns into a failure.
//!
//! Anything else is [`CodegenError::Unsupported`], with the construct named, so that `una build`
//! reports what the native backend cannot compile yet instead of building something else.
//!
//! Values are machine words, as the runtime describes them (`una-rt`, `values`). The object's
//! C `main` calls `una_rt_start` of the runtime with the process arguments, the program's
//! description (`spec/16 §2`) and `una_main`, which runs `main` of Una. This crate emits
//! machine code but never runs it: there is no JIT, and no `unsafe`.
#![deny(missing_docs)]

mod lower;

use lower::Body;

use std::collections::BTreeMap;
use std::fmt;

use cranelift_codegen::ir::{AbiParam, InstBuilder, UserFuncName, types};
use cranelift_codegen::isa::{self, OwnedTargetIsa};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::{Context, ir};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module, default_libcall_names};
use cranelift_object::{ObjectBuilder, ObjectModule};
use target_lexicon::Triple;
use una_core::{Decl, Expr, Fun, Name, Path, Program, Type};
use una_syntax::Span;

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

/// The object file of the program: `main` of module `entry` and the functions it reaches,
/// compiled for the host target. `meta` gives the description of the program that the runtime
/// reads at start (`spec/16 §2`), from the spans of the program's calls, in the order of the
/// indices the generated code passes to the runtime on each call.
pub fn compile(
    program: &Program,
    entry: &str,
    meta: &dyn Fn(&[Span]) -> String,
) -> Result<Vec<u8>, CodegenError> {
    let main = function(program, entry, "main").ok_or(CodegenError::NoMain)?;
    let mut emitter = Emitter::new(program, isa()?)?;
    let (main_id, _) = emitter.function(&Path {
        module: entry.to_owned(),
        name: "main".to_owned(),
    })?;
    emitter.drain()?;
    let meta = meta(&emitter.sites);
    emitter.entry(main_id, main, &meta)?;
    emitter
        .module
        .finish()
        .emit()
        .map_err(|error| CodegenError::Internal(error.to_string()))
}

/// The function `name` of module `module`.
fn function<'p>(program: &'p Program, module: &str, name: &str) -> Option<&'p Fun> {
    program
        .modules
        .iter()
        .filter(|m| m.path == module)
        .flat_map(|m| &m.decls)
        .find_map(|decl| match decl {
            Decl::Fun(fun) if fun.name == name => Some(fun),
            _ => None,
        })
}

/// The body of the function or predicate `name` of module `module`: `None` when there is none,
/// and an error when the backend does not compile it yet.
fn callee<'p>(
    program: &'p Program,
    module: &str,
    name: &str,
) -> Option<Result<Body<'p>, CodegenError>> {
    program
        .modules
        .iter()
        .filter(|m| m.path == module)
        .flat_map(|m| &m.decls)
        .find_map(|decl| match decl {
            Decl::Fun(fun) if fun.name == name => Some(Ok(Body {
                closure: false,
                captures: Vec::new(),
                params: &fun.params,
                result: Some(&fun.result),
                fails: fun.fails.is_some(),
                body: &fun.body,
            })),
            Decl::Pred {
                name: pred,
                params,
                body,
                ..
            } if pred == name => Some(Ok(Body {
                closure: false,
                captures: Vec::new(),
                params,
                result: None,
                fails: false,
                body,
            })),
            _ => None,
        })
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

/// The runtime functions the generated code calls, with their arities: every parameter and
/// result is a word (`i64`), and `false` is no result.
const RUNTIME: &[(&str, usize, bool)] = &[
    ("una_rt_int_literal", 2, true),
    ("una_rt_text_literal", 2, true),
    ("una_rt_int_add", 2, true),
    ("una_rt_int_sub", 2, true),
    ("una_rt_int_mul", 2, true),
    ("una_rt_int_div", 2, true),
    ("una_rt_int_mod", 2, true),
    ("una_rt_int_neg", 1, true),
    ("una_rt_int_compare", 2, true),
    ("una_rt_int_text", 1, true),
    ("una_rt_bool_text", 1, true),
    ("una_rt_text_concat", 2, true),
    ("una_rt_text_equal", 2, true),
    ("una_rt_text_compare", 2, true),
    ("una_rt_text_len", 1, true),
    ("una_rt_enter", 1, false),
    ("una_rt_leave", 0, false),
    ("una_rt_console_print", 1, false),
    ("una_rt_retain", 1, false),
    ("una_rt_release", 1, false),
    ("una_rt_push", 2, false),
    ("una_rt_data", 1, true),
    ("una_rt_list", 0, true),
    ("una_rt_closure", 1, true),
    ("una_rt_field", 2, true),
    ("una_rt_tag", 1, true),
    ("una_rt_closure_code", 1, true),
    ("una_rt_count", 1, true),
    ("una_rt_list_len", 1, true),
    ("una_rt_equal", 2, true),
    ("una_rt_abs", 1, true),
    ("una_rt_min", 2, true),
    ("una_rt_max", 2, true),
    ("una_rt_is_digits", 1, true),
    ("una_rt_starts_with", 2, true),
    ("una_rt_trim", 1, true),
    ("una_rt_to_int", 1, true),
    ("una_rt_is_empty", 1, true),
    ("una_rt_first", 1, true),
    ("una_rt_at", 2, true),
    ("una_rt_append", 2, true),
    ("una_rt_contains", 2, true),
    ("una_rt_sum", 1, true),
    ("una_rt_or_else", 2, true),
    ("una_rt_main_failed", 1, false),
];

/// The object being built.
struct Emitter<'p> {
    program: &'p Program,
    module: ObjectModule,
    /// The runtime functions, by name.
    runtime: BTreeMap<&'static str, FuncId>,
    /// The compiled functions, by qualified name, with whether they can fail, and the bodies
    /// still to compile.
    functions: BTreeMap<String, (FuncId, bool)>,
    pending: Vec<(FuncId, Body<'p>)>,
    /// The number of lambdas declared so far.
    lambdas: usize,
    /// Read-only data: text literals and integer literals beyond the tagged range.
    data: BTreeMap<String, DataId>,
    /// The spans of the calls, by the index passed to `una_rt_enter`.
    sites: Vec<Span>,
}

impl<'p> Emitter<'p> {
    fn new(program: &'p Program, isa: OwnedTargetIsa) -> Result<Emitter<'p>, CodegenError> {
        let builder = ObjectBuilder::new(isa, "una", default_libcall_names()).map_err(internal)?;
        let mut module = ObjectModule::new(builder);
        let mut runtime = BTreeMap::new();
        for &(name, params, returns) in RUNTIME {
            let mut signature = module.make_signature();
            for _ in 0..params {
                signature.params.push(AbiParam::new(types::I64));
            }
            if returns {
                signature.returns.push(AbiParam::new(types::I64));
            }
            let id = module
                .declare_function(name, Linkage::Import, &signature)
                .map_err(internal)?;
            runtime.insert(name, id);
        }
        Ok(Emitter {
            program,
            module,
            runtime,
            functions: BTreeMap::new(),
            pending: Vec::new(),
            lambdas: 0,
            data: BTreeMap::new(),
            sites: Vec::new(),
        })
    }

    /// The function or predicate `path` of the package, declared now and compiled by
    /// [`Emitter::drain`].
    fn function(&mut self, path: &Path) -> Result<(FuncId, bool), CodegenError> {
        let qualified = format!("{}.{}", path.module, path.name);
        if let Some(found) = self.functions.get(&qualified) {
            return Ok(*found);
        }
        let body = callee(self.program, &path.module, &path.name).ok_or_else(|| {
            CodegenError::Unsupported(format!(
                "calls of `{qualified}`, which is not a function of the package"
            ))
        })??;
        let name = format!("una_fn_{}", self.functions.len());
        let id = self.declare(&name, body.params.len())?;
        let fails = body.fails;
        self.functions.insert(qualified, (id, fails));
        self.pending.push((id, body));
        Ok((id, fails))
    }

    /// The code of a closure of the lambda `params` to `body`, which captures `captures`:
    /// declared now, and compiled by [`Emitter::drain`].
    fn lambda(
        &mut self,
        params: &'p [(Name, Type)],
        body: &'p Expr,
        captures: Vec<(Name, Type)>,
    ) -> Result<FuncId, CodegenError> {
        let name = format!("una_lambda_{}", self.lambdas);
        self.lambdas += 1;
        // The closure itself comes first.
        let id = self.declare(&name, params.len() + 1)?;
        self.pending.push((
            id,
            Body {
                closure: true,
                captures,
                params,
                result: None,
                fails: false,
                body,
            },
        ));
        Ok(id)
    }

    /// A local function of `params` words that returns a word and whether it failed.
    fn declare(&mut self, name: &str, params: usize) -> Result<FuncId, CodegenError> {
        let mut signature = self.module.make_signature();
        for _ in 0..params {
            signature.params.push(AbiParam::new(types::I64));
        }
        signature.returns.push(AbiParam::new(types::I64));
        signature.returns.push(AbiParam::new(types::I64));
        self.module
            .declare_function(name, Linkage::Local, &signature)
            .map_err(internal)
    }

    /// Compiles every declared function, and those they call in turn.
    fn drain(&mut self) -> Result<(), CodegenError> {
        while let Some((id, body)) = self.pending.pop() {
            let mut context = self.module.make_context();
            context.func.signature = self
                .module
                .declarations()
                .get_function_decl(id)
                .signature
                .clone();
            context.func.name = UserFuncName::user(0, id.as_u32());
            let mut builder_context = FunctionBuilderContext::new();
            let config = self.module.target_config();
            {
                let builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
                lower::function(self, builder, &body, config)?;
            }
            self.define(id, &mut context)?;
        }
        Ok(())
    }

    /// The read-only data of `bytes`, emitted once per distinct content.
    fn bytes(&mut self, bytes: &str) -> Result<DataId, CodegenError> {
        if let Some(id) = self.data.get(bytes) {
            return Ok(*id);
        }
        let name = format!("una_data_{}", self.data.len());
        let id = self
            .module
            .declare_data(&name, Linkage::Local, false, false)
            .map_err(internal)?;
        let mut data = DataDescription::new();
        data.define(bytes.as_bytes().to_vec().into_boxed_slice());
        self.module.define_data(id, &data).map_err(internal)?;
        self.data.insert(bytes.to_owned(), id);
        Ok(id)
    }

    /// The C `main`, which hands the process to the runtime with `una_main`, the compiled `main`
    /// of Una wrapped to return 0.
    fn entry(&mut self, main_id: FuncId, main: &Fun, meta: &str) -> Result<(), CodegenError> {
        let pointer = self.module.target_config().pointer_type();
        let call_conv = self.module.target_config().default_call_conv;
        let config = self.module.target_config();

        // `una_main`: calls `main` of Una, whose result is `unit`, and returns 0, or reports its
        // failure (U6001) and returns 1.
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
            debug_assert!(main.params.is_empty());
            let main_ref = self.module.declare_func_in_func(main_id, builder.func);
            let call = builder.ins().call(main_ref, &[]);
            let results = builder.inst_results(call).to_vec();
            let (Some(&value), Some(&failed)) = (results.first(), results.get(1)) else {
                return Err(internal("`main` lost its result"));
            };
            let returned = builder.create_block();
            let failing = builder.create_block();
            builder.ins().brif(failed, failing, &[], returned, &[]);
            builder.switch_to_block(returned);
            builder.seal_block(returned);
            let zero = builder.ins().iconst(types::I64, 0);
            builder.ins().return_(&[zero]);
            builder.switch_to_block(failing);
            builder.seal_block(failing);
            let report = *self
                .runtime
                .get("una_rt_main_failed")
                .ok_or_else(|| internal("the runtime lacks `una_rt_main_failed`"))?;
            let report = self.module.declare_func_in_func(report, builder.func);
            builder.ins().call(report, &[value]);
            let one = builder.ins().iconst(types::I64, 1);
            builder.ins().return_(&[one]);
            builder.finalize(config);
        }
        self.define(una_main_id, &mut context)?;

        let meta_id = self.bytes(meta)?;
        let mut start = self.module.make_signature();
        start.params.push(AbiParam::new(types::I32));
        for _ in 0..4 {
            start.params.push(AbiParam::new(pointer));
        }
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
            builder.finalize(config);
        }
        self.define(c_main_id, &mut context)
    }

    fn define(&mut self, id: FuncId, context: &mut Context) -> Result<(), CodegenError> {
        self.module
            .define_function(id, context)
            .map_err(|error| internal(format!("{error:?}")))?;
        self.module.clear_context(context);
        Ok(())
    }
}
