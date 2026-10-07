//! Native code generation (`spec/16`, ADR 0055): Core IR of a checked and verified package to an
//! object file for the host target, through Cranelift. `una build` links the object with the
//! runtime library `una-rt` through the system `cc`.
//!
//! The backend is delivered in slices (#26). It compiles `main` of the entry module and every
//! function it reaches, when they use what the backend compiles so far:
//! - integers (tagged words, promoted to arbitrary precision by the runtime, NB-3), `Bool`,
//!   `Unit`, and `Text` literals, interpolation and comparison;
//! - fixed-width integers of edition 0.2, as integer words whose overflow is proven absent, and
//!   their wrapping, saturating and bit methods (`spec/18`);
//! - machine widths (NB-3, FW-1): where the operands and the result of `+`, `-`, `*` or a
//!   comparison are proven to fit a tagged word, the code is a plain machine instruction with no
//!   check (`range`);
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
//! - capabilities, passed to each function after its parameters (`spec/16 §4` rule 4): host
//!   instances, which the runtime serves with the interpreter's grants and path rules, and
//!   handler instances, which `with` installs and whose operations are compiled code;
//! - traces: every operation records its event through the runtime, with its arguments, result
//!   and failure rendered and redacted as the interpreter does;
//! - edition 0.2's memory model: tuples, `let (x, ...)`, and `for` loops, whose iterations
//!   check the memory budget (ADR 0061); and effectful and failing functions in the prelude's
//!   higher-order functions, whose failure stops the loop (ADR 0062).
//!
//! Examples run in the interpreter only, as `una test` does (#115).
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

mod describe;
mod lower;
mod owned;
mod range;

pub use range::{machine_width, proven_range};

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
use una_core::{CapOp, Constraint, Decl, Expr, Fun, Name, Path, Program, Type};
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

/// What the generated code refers to by index, which the runtime reads at start: `una build`
/// embeds it in the program's description (`spec/16 §2`).
#[derive(Clone, Copy, Debug)]
pub struct Described<'a> {
    /// The spans of the program's calls, by the index passed to `una_rt_enter`.
    pub sites: &'a [Span],
    /// The types that the runtime renders, by index, as `una-rt`'s `render` reads them.
    pub types: &'a [serde_json::Value],
    /// The operations of the program, by the index passed to `una_rt_event`: the capability,
    /// the operation, its parameters' names, and the type and redaction of its arguments,
    /// result and failure.
    pub ops: &'a [serde_json::Value],
    /// The type of `main`'s failures, when it can fail.
    pub failure: Option<usize>,
}

/// The object file of the program: `main` of module `entry` and the functions it reaches,
/// compiled for the host target. `meta` gives the description of the program that the runtime
/// reads at start (`spec/16 §2`), from what the generated code refers to by index.
pub fn compile(
    program: &Program,
    entry: &str,
    meta: &dyn Fn(&Described<'_>) -> String,
) -> Result<Vec<u8>, CodegenError> {
    let main = function(program, entry, "main").ok_or(CodegenError::NoMain)?;
    let mut emitter = Emitter::new(program, isa()?)?;
    // U6001 shows `main`'s failure by the kind the checker resolved: without it, or with a kind
    // it cannot describe, a guess could show a labeled field.
    let failure = match (&main.fails, &main.error) {
        (None, _) => None,
        (Some(_), Some(kind)) => {
            let root = emitter.types.of(&una_core::Type::Base {
                base: una_core::Base::Named {
                    module: kind.module.clone(),
                    name: kind.name.clone(),
                    args: Vec::new(),
                },
                refinements: Vec::new(),
                label: una_core::Label::public(),
            });
            if emitter.types.unknown() {
                return Err(CodegenError::Internal(
                    "the error kind of `main` holds a kind that has no declaration".to_owned(),
                ));
            }
            Some(root)
        }
        (Some(_), None) => {
            return Err(CodegenError::Internal(
                "the error kind of `main` is not resolved".to_owned(),
            ));
        }
    };
    let main_id = emitter.function(&Path {
        module: entry.to_owned(),
        name: "main".to_owned(),
    })?;
    emitter.drain()?;
    let meta = meta(&Described {
        sites: &emitter.sites,
        types: emitter.types.shapes(),
        ops: &emitter.ops,
        failure,
    });
    emitter.entry(main_id.id, main, &meta)?;
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
            Decl::Fun(fun) if fun.name == name => Some(fun.as_ref()),
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
                this: None,
                // After the parameters, the type descriptor of each constrained type parameter
                // (`spec/20 §7`), bound to `$T` for the parameter `T`.
                params: fun
                    .params
                    .iter()
                    .cloned()
                    .chain(
                        fun.type_params
                            .iter()
                            .zip(&fun.constraints)
                            .filter(|(_, constraint)| **constraint != Constraint::None)
                            .map(|(param, _)| {
                                (lower::descriptor_name(param), lower::descriptor_type())
                            }),
                    )
                    .collect(),
                constraints: fun.constraints.clone(),
                // Set by `Emitter::function` from the package's borrow inference.
                owned: Vec::new(),
                uses: fun.uses.iter().map(|u| u.cap.clone()).collect(),
                result: Some(fun.result.clone()),
                exact: false,
                body: &fun.body,
            })),
            Decl::Pred {
                name: pred,
                params,
                body,
                reflect,
                ..
            } if pred == name => Some(Ok(Body {
                this: None,
                constraints: Vec::new(),
                owned: Vec::new(),
                params: params
                    .iter()
                    .map(|(name, ty)| (name.clone(), range::unrefined(ty)))
                    .collect(),
                uses: Vec::new(),
                result: None,
                // The verifier defines a reflect predicate by its body as a term, exact, and
                // makes no overflow obligation in it (#180).
                exact: *reflect,
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
    ("una_rt_text_append_owned", 2, true),
    ("una_rt_text_equal", 2, true),
    ("una_rt_text_compare", 2, true),
    ("una_rt_text_len", 1, true),
    ("una_rt_enter", 1, true),
    ("una_rt_leave", 0, false),
    ("una_rt_console_print", 1, true),
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
    ("una_rt_set", 3, true),
    ("una_rt_append_owned", 2, true),
    ("una_rt_set_owned", 3, true),
    ("una_rt_map_empty", 0, true),
    ("una_rt_map_len", 1, true),
    ("una_rt_map_has", 2, true),
    ("una_rt_map_get", 2, true),
    ("una_rt_map_with", 3, true),
    ("una_rt_map_with_owned", 3, true),
    ("una_rt_map_without", 2, true),
    ("una_rt_map_without_owned", 2, true),
    ("una_rt_map_list", 2, true),
    ("una_rt_split", 2, true),
    ("una_rt_join", 2, true),
    ("una_rt_ends_with", 2, true),
    ("una_rt_contains_text", 2, true),
    ("una_rt_find", 2, true),
    ("una_rt_replace", 3, true),
    ("una_rt_slice", 3, true),
    ("una_rt_to_upper", 1, true),
    ("una_rt_to_lower", 1, true),
    ("una_rt_chars", 1, true),
    ("una_rt_trim_start", 1, true),
    ("una_rt_trim_end", 1, true),
    ("una_rt_bytes", 1, true),
    ("una_rt_parse_failure", 1, true),
    ("una_rt_parse_int", 1, true),
    ("una_rt_reuse", 2, true),
    ("una_rt_sum", 1, true),
    ("una_rt_or_else", 2, true),
    ("una_rt_main_failed", 1, false),
    ("una_rt_op_code", 2, true),
    ("una_rt_fixed", 4, true),
    ("una_rt_budget", 1, true),
    ("una_rt_poll", 0, true),
    ("una_rt_generic_compare", 3, true),
    ("una_rt_generic_equal", 3, true),
    ("una_rt_generic_to_int", 2, true),
    ("una_rt_generic_min", 3, true),
    ("una_rt_generic_max", 3, true),
    ("una_rt_generic_contains", 3, true),
    ("una_rt_generic_sum", 2, true),
    ("una_rt_generic_sort", 2, true),
    ("una_rt_generic_write", 2, true),
    ("una_rt_write", 2, true),
    ("una_rt_descriptor_composite", 2, true),
    ("una_rt_operation_begin", 0, false),
    ("una_rt_operation_end", 0, false),
    ("una_rt_together", 2, true),
    ("una_rt_together_for", 3, true),
    ("una_rt_together_status", 0, true),
    ("una_rt_range", 3, true),
    ("una_rt_clock_now", 0, true),
    ("una_rt_env_get", 1, true),
    ("una_rt_files_read", 1, true),
    ("una_rt_files_write", 2, true),
    ("una_rt_keys_hmac", 2, true),
    ("una_rt_http_get", 3, true),
    ("una_rt_http_post", 4, true),
    ("una_rt_http_put", 4, true),
    ("una_rt_http_delete", 3, true),
    ("una_rt_host_failed", 0, true),
    ("una_rt_parse_json", 1, true),
    ("una_rt_to_json", 1, true),
    ("una_rt_quote_json", 1, true),
    ("una_rt_is_json_number", 1, true),
    ("una_rt_host_instance", 1, true),
    ("una_rt_handler_instance", 1, true),
    ("una_rt_event", 2, true),
    ("una_rt_push_op", 2, false),
    ("una_rt_event_arg", 4, false),
    ("una_rt_event_end", 4, false),
];

/// A compiled function: its code, whether it can fail, and the capabilities it takes after its
/// parameters, by name.
#[derive(Clone, Debug)]
struct Callee {
    id: FuncId,
    uses: Vec<Name>,
    /// The constraint of each type parameter: each one but `None` takes a type descriptor
    /// after the parameters (`spec/20 §7`).
    constraints: Vec<Constraint>,
    /// Which parameters the function owns (ADR 0072 D2): the caller gives up or retains a
    /// reference for each of them. Empty when every parameter is borrowed.
    owned: Vec<bool>,
}

/// A handler of the package.
#[derive(Clone, Copy)]
struct Handler<'p> {
    params: &'p [(Name, Type)],
    capability: &'p Path,
    uses: &'p [una_core::Use],
    ops: &'p [una_core::HandlerOp],
}

/// The object being built.
struct Emitter<'p> {
    program: &'p Program,
    module: ObjectModule,
    /// The runtime functions, by name.
    runtime: BTreeMap<&'static str, FuncId>,
    /// The compiled functions, by qualified name, with whether they can fail and the
    /// capabilities they take, and the bodies still to compile.
    functions: BTreeMap<String, Callee>,
    /// The owned parameters of each function of the package (ADR 0072 D2).
    owned: BTreeMap<String, Vec<bool>>,
    pending: Vec<(FuncId, Body<'p>)>,
    /// The number of lambdas declared so far.
    lambdas: usize,
    /// Read-only data: text literals and integer literals beyond the tagged range.
    data: BTreeMap<String, DataId>,
    /// The spans of the calls, by the index passed to `una_rt_enter`.
    sites: Vec<Span>,
    /// The types the runtime renders.
    types: describe::Types<'p>,
    /// The operations, by the index passed to `una_rt_event`.
    ops: Vec<serde_json::Value>,
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
            owned: owned::owned_params(program),
            pending: Vec::new(),
            lambdas: 0,
            data: BTreeMap::new(),
            sites: Vec::new(),
            types: describe::Types::new(program),
            ops: Vec::new(),
        })
    }

    /// The function or predicate `path` of the package, declared now and compiled by
    /// [`Emitter::drain`].
    fn function(&mut self, path: &Path) -> Result<Callee, CodegenError> {
        let qualified = format!("{}.{}", path.module, path.name);
        if let Some(found) = self.functions.get(&qualified) {
            return Ok(found.clone());
        }
        let body = callee(self.program, &path.module, &path.name).ok_or_else(|| {
            CodegenError::Unsupported(format!(
                "calls of `{qualified}`, which is not a function of the package"
            ))
        })??;
        let mut body = body;
        body.owned = self.owned.get(&qualified).cloned().unwrap_or_default();
        let name = format!("una_fn_{}", self.functions.len());
        let id = self.declare(&name, body.params.len() + body.uses.len())?;
        let callee = Callee {
            id,
            uses: body.uses.clone(),
            constraints: body.constraints.clone(),
            owned: body.owned.clone(),
        };
        self.functions.insert(qualified, callee.clone());
        self.pending.push((id, body));
        Ok(callee)
    }

    /// The code of a closure of the lambda `params` to `body`, which captures the values
    /// `captures` and the capabilities `caps`: declared now, and compiled by
    /// [`Emitter::drain`].
    fn lambda(
        &mut self,
        params: &'p [(Name, Type)],
        body: &'p Expr,
        captures: Vec<(Name, Type)>,
        caps: Vec<Name>,
        exact: bool,
    ) -> Result<FuncId, CodegenError> {
        let name = format!("una_lambda_{}", self.lambdas);
        self.lambdas += 1;
        // The closure itself comes first.
        let id = self.declare(&name, params.len() + 1)?;
        self.pending.push((
            id,
            Body {
                this: Some((captures, caps)),
                constraints: Vec::new(),
                owned: Vec::new(),
                // A lambda's refinements are not proven of its arguments (`range`).
                params: params
                    .iter()
                    .map(|(name, ty)| (name.clone(), range::unrefined(ty)))
                    .collect(),
                uses: Vec::new(),
                result: None,
                exact,
                body,
            },
        ));
        Ok(id)
    }

    /// The capability declared at `path`, by its operations: a host capability of
    /// [`una_core::HOST_MODULE`], or one of the package.
    fn capability(&self, path: &Path) -> Option<&'p [CapOp]> {
        let program = self.program;
        let decls: Vec<&'p Decl> = if path.module == una_core::HOST_MODULE {
            program.host.iter().collect()
        } else {
            program
                .modules
                .iter()
                .filter(|m| m.path == path.module)
                .flat_map(|m| &m.decls)
                .collect()
        };
        decls.into_iter().find_map(|decl| match decl {
            Decl::Cap { name, ops } if *name == path.name => Some(ops.as_slice()),
            _ => None,
        })
    }

    /// The handler declared at `path`.
    fn handler(&self, path: &Path) -> Option<Handler<'p>> {
        self.program
            .modules
            .iter()
            .filter(|m| m.path == path.module)
            .flat_map(|m| &m.decls)
            .find_map(|decl| match decl {
                Decl::Handler {
                    name,
                    params,
                    capability_path,
                    uses,
                    ops,
                    ..
                } if *name == path.name => Some(Handler {
                    params,
                    capability: capability_path,
                    uses,
                    ops,
                }),
                _ => None,
            })
    }

    /// The code of operation `op` of the handler at `path`, whose first parameter is the
    /// instance: declared now, and compiled by [`Emitter::drain`].
    fn handler_op(&mut self, path: &Path, op: &CapOp) -> Result<FuncId, CodegenError> {
        let qualified = format!("{}.{}.{}", path.module, path.name, op.name);
        if let Some(found) = self.functions.get(&qualified) {
            return Ok(found.id);
        }
        let handler = self
            .handler(path)
            .ok_or_else(|| internal(format!("no handler `{}`", path.qualified())))?;
        let implementation = handler
            .ops
            .iter()
            .find(|o| o.name == op.name)
            .ok_or_else(|| internal(format!("`{}` lacks `{}`", path.qualified(), op.name)))?;
        let params: Vec<(Name, Type)> = implementation
            .params
            .iter()
            .zip(&op.params)
            .map(|(name, (_, ty))| (name.clone(), range::unrefined(ty)))
            .collect();
        let name = format!("una_op_{}", self.functions.len());
        let id = self.declare(&name, params.len() + 1)?;
        self.functions.insert(
            qualified,
            Callee {
                id,
                uses: Vec::new(),
                constraints: Vec::new(),
                owned: Vec::new(),
            },
        );
        self.pending.push((
            id,
            Body {
                constraints: Vec::new(),
                owned: Vec::new(),
                this: Some((
                    handler
                        .params
                        .iter()
                        .map(|(name, ty)| (name.clone(), range::unrefined(ty)))
                        .collect(),
                    handler.uses.iter().map(|u| u.cap.clone()).collect(),
                )),
                params,
                uses: Vec::new(),
                result: Some(op.result.clone()),
                exact: false,
                body: &implementation.body,
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
            // A host instance for each capability `main` uses (`spec/11 §2`), which `main` takes
            // after its parameters.
            let mut instances = Vec::new();
            for need in &main.uses {
                let name = self.bytes(&need.capability)?;
                let name = self.module.declare_data_in_func(name, builder.func);
                let address = builder.ins().symbol_value(types::I64, name);
                let len = i64::try_from(need.capability.len()).map_err(internal)?;
                let len = builder.ins().iconst(types::I64, len);
                let text =
                    self.runtime_call(&mut builder, "una_rt_text_literal", &[address, len])?;
                let instance = self.runtime_call(&mut builder, "una_rt_host_instance", &[text])?;
                instances.push(instance);
            }
            let main_ref = self.module.declare_func_in_func(main_id, builder.func);
            let call = builder.ins().call(main_ref, &instances);
            let results = builder.inst_results(call).to_vec();
            for &instance in &instances {
                self.runtime_call(&mut builder, "una_rt_release", &[instance])?;
            }
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

    /// Calls the runtime function `name` in `builder`'s function; its result, or 0 when it has
    /// none.
    fn runtime_call(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        name: &str,
        args: &[ir::Value],
    ) -> Result<ir::Value, CodegenError> {
        let id = *self
            .runtime
            .get(name)
            .ok_or_else(|| internal(format!("the runtime lacks `{name}`")))?;
        let callee = self.module.declare_func_in_func(id, builder.func);
        let call = builder.ins().call(callee, args);
        Ok(match builder.inst_results(call).first() {
            Some(&value) => value,
            None => builder.ins().iconst(types::I64, 0),
        })
    }

    fn define(&mut self, id: FuncId, context: &mut Context) -> Result<(), CodegenError> {
        self.module
            .define_function(id, context)
            .map_err(|error| internal(format!("{error:?}")))?;
        self.module.clear_context(context);
        Ok(())
    }
}
