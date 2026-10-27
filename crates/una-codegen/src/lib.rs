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
    /// Whether the compiled code makes a hash table, so that the run reads the process key of
    /// keyed hashing before `main` (ADR 0119 rule 9). The code compiled is the code `main`
    /// reaches, the standard library's included.
    pub hashing: bool,
    /// Whether the compiled code holds a task construct or a `for ever`, so the runtime starts
    /// its workers and helper pools before `main` (ADR 0116 amendment, P8; #1145).
    pub threads: bool,
}

/// The object file of the program: `main` of module `entry` and the functions it reaches,
/// compiled for the host target. `meta` gives the description of the program that the runtime
/// reads at start (`spec/16 §2`), from what the generated code refers to by index.
pub fn compile(
    program: &Program,
    entry: &str,
    meta: &dyn Fn(&Described<'_>) -> String,
) -> Result<Vec<u8>, CodegenError> {
    compile_start(program, entry, false, meta)
}

/// The object file of a driven program (ADR 0116 rule 3): `init`, `step` and `on_stop` of
/// module `module`, the functions they reach, and the host's loop, which asks the runtime for
/// each tick. `una build` checked the functions' shapes. A failure of `init` is U6001, as one
/// of `main` is, and `meta`'s `failure` describes it.
pub fn compile_drive(
    program: &Program,
    module: &str,
    meta: &dyn Fn(&Described<'_>) -> String,
) -> Result<Vec<u8>, CodegenError> {
    compile_start(program, module, true, meta)
}

/// The object file that starts at `main` of `entry`, or with `driven` at its `init`.
fn compile_start(
    program: &Program,
    entry: &str,
    driven: bool,
    meta: &dyn Fn(&Described<'_>) -> String,
) -> Result<Vec<u8>, CodegenError> {
    let first = if driven { "init" } else { "main" };
    let main = function(program, entry, first).ok_or(CodegenError::NoMain)?;
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
    let at = |name: &str| Path {
        module: entry.to_owned(),
        name: name.to_owned(),
    };
    let main_id = emitter.function(&at(first))?;
    let loop_functions = if driven {
        let step = function(program, entry, "step").ok_or(CodegenError::NoMain)?;
        let step_id = emitter.function(&at("step"))?.id;
        let on_stop = match function(program, entry, "on_stop") {
            Some(on_stop) => Some((on_stop, emitter.function(&at("on_stop"))?.id)),
            None => None,
        };
        Some(((step, step_id), on_stop))
    } else {
        None
    };
    emitter.drain()?;
    // #407: a type described for printing or comparison names a kind that nothing declares,
    // a bug of elaboration; it would print as `?` rather than as its declaration says.
    if emitter.types.unknown() {
        return Err(CodegenError::Internal(
            "a type of the program holds a kind that has no declaration".to_owned(),
        ));
    }
    let meta = meta(&Described {
        sites: &emitter.sites,
        types: emitter.types.shapes(),
        ops: &emitter.ops,
        failure,
        hashing: emitter.hashing,
        threads: emitter.threads,
    });
    match loop_functions {
        None => emitter.entry(main_id.id, main, &meta)?,
        Some((step, on_stop)) => {
            let owned = Owned {
                step: emitter.owned(&at("step")),
                on_stop: emitter.owned(&at("on_stop")),
            };
            emitter.drive_entry((main_id.id, main), step, on_stop, &owned, &meta)?;
        }
    }
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
    ("una_rt_int_text", 2, true),
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
    ("una_rt_closure", 2, true),
    ("una_rt_field", 2, true),
    ("una_rt_tag", 1, true),
    ("una_rt_closure_code", 1, true),
    ("una_rt_count", 1, true),
    ("una_rt_list_len", 1, true),
    ("una_rt_equal_at", 3, true),
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
    ("una_rt_contains_at", 3, true),
    ("una_rt_set", 4, true),
    ("una_rt_append_owned", 2, true),
    ("una_rt_set_owned", 4, true),
    ("una_rt_map_empty", 0, true),
    ("una_rt_map_len", 1, true),
    ("una_rt_map_has", 2, true),
    ("una_rt_map_get", 2, true),
    ("una_rt_map_with", 3, true),
    ("una_rt_map_with_owned", 3, true),
    ("una_rt_map_without", 2, true),
    ("una_rt_map_without_owned", 2, true),
    ("una_rt_map_list", 2, true),
    ("una_rt_collection_len", 1, true),
    ("una_rt_seq_empty", 0, true),
    ("una_rt_seq_of", 2, true),
    ("una_rt_seq_op", 3, true),
    ("una_rt_seq_op_owned", 3, true),
    ("una_rt_set_empty", 0, true),
    ("una_rt_set_of", 1, true),
    ("una_rt_set_op", 4, true),
    ("una_rt_set_op_owned", 4, true),
    ("una_rt_priority_empty", 1, true),
    ("una_rt_priority_op", 4, true),
    ("una_rt_priority_op_owned", 4, true),
    ("una_rt_bit_set_empty", 2, true),
    ("una_rt_bit_set_op", 4, true),
    ("una_rt_bit_set_op_owned", 4, true),
    ("una_rt_hash_empty", 2, true),
    ("una_rt_hash_of_list", 4, true),
    ("una_rt_hash_map_op", 6, true),
    ("una_rt_hash_map_op_owned", 6, true),
    ("una_rt_hash_set_op", 5, true),
    ("una_rt_hash_set_op_owned", 5, true),
    ("una_rt_split", 2, true),
    ("una_rt_join", 2, true),
    ("una_rt_ends_with", 2, true),
    ("una_rt_contains_text", 2, true),
    ("una_rt_find", 2, true),
    ("una_rt_replace", 4, true),
    ("una_rt_slice", 4, true),
    ("una_rt_to_upper", 1, true),
    ("una_rt_to_lower", 1, true),
    ("una_rt_chars", 1, true),
    ("una_rt_trim_start", 1, true),
    ("una_rt_trim_end", 1, true),
    ("una_rt_bytes", 1, true),
    ("una_rt_parse_failure", 1, true),
    ("una_rt_parse_int", 1, true),
    ("una_rt_f64_literal", 1, true),
    ("una_rt_f64_op", 4, true),
    ("una_rt_f64_from_bits", 1, true),
    ("una_rt_parse_f64_failure", 1, true),
    ("una_rt_parse_f64", 1, true),
    ("una_rt_as_f64", 1, true),
    ("una_rt_json_number", 1, true),
    ("una_rt_reuse", 2, true),
    ("una_rt_sum", 1, true),
    ("una_rt_or_else", 2, true),
    ("una_rt_main_failed", 1, false),
    ("una_rt_drive_tick", 0, true),
    ("una_rt_ever_start", 0, true),
    ("una_rt_ever_end", 0, false),
    ("una_rt_op_code", 2, true),
    ("una_rt_fixed", 4, true),
    ("una_rt_decimal", 5, true),
    ("una_rt_parse_decimal_failure", 2, true),
    ("una_rt_parse_decimal", 2, true),
    ("una_rt_as_decimal", 2, true),
    ("una_rt_budget", 1, true),
    ("una_rt_deadline_enter", 1, false),
    ("una_rt_deadline_iteration", 0, false),
    ("una_rt_deadline_leave", 0, false),
    ("una_rt_within_enter", 1, true),
    ("una_rt_within_end", 2, true),
    ("una_rt_poll", 0, true),
    ("una_rt_stopped", 0, true),
    ("una_rt_generic_compare", 3, true),
    ("una_rt_generic_equal_at", 4, true),
    ("una_rt_generic_to_int", 2, true),
    ("una_rt_generic_min", 3, true),
    ("una_rt_generic_max", 3, true),
    ("una_rt_generic_contains_at", 4, true),
    ("una_rt_generic_sum", 2, true),
    ("una_rt_generic_sort", 2, true),
    ("una_rt_generic_write", 3, true),
    ("una_rt_write", 3, true),
    ("una_rt_descriptor_composite", 2, true),
    ("una_rt_operation_begin", 0, false),
    ("una_rt_operation_end", 0, false),
    ("una_rt_together", 2, true),
    ("una_rt_together_for", 3, true),
    ("una_rt_first_of", 2, true),
    ("una_rt_together_status", 0, true),
    ("una_rt_tasks_begin", 3, true),
    ("una_rt_start_task", 2, true),
    ("una_rt_tasks_end", 1, false),
    ("una_rt_range", 4, true),
    ("una_rt_clock_now", 0, true),
    ("una_rt_env_get", 2, true),
    ("una_rt_files_read", 2, true),
    ("una_rt_files_write", 2, true),
    ("una_rt_keys_hmac", 2, true),
    ("una_rt_http_enable", 0, false),
    ("una_rt_http_get", 4, true),
    ("una_rt_http_post", 5, true),
    ("una_rt_http_put", 5, true),
    ("una_rt_http_delete", 4, true),
    ("una_rt_host_failed", 0, true),
    ("una_rt_parse_json", 2, true),
    ("una_rt_to_json", 2, true),
    ("una_rt_json_stopped", 0, true),
    ("una_rt_quote_json", 1, true),
    ("una_rt_is_json_number", 1, true),
    ("una_rt_host_instance", 1, true),
    ("una_rt_handler_instance", 1, true),
    ("una_rt_limiter_instance", 3, true),
    ("una_rt_limit_acquire", 2, true),
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
}

/// Which parameters of `step` and `on_stop` of a driven module are owned (ADR 0072 D2).
struct Owned {
    step: Vec<bool>,
    on_stop: Vec<bool>,
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
    /// Whether a compiled function makes a hash table (ADR 0119 rule 9).
    hashing: bool,
    /// Whether a task construct or a `for ever` was compiled ([`Described::threads`]).
    threads: bool,
    /// Whether an operation of `Http` was compiled: the C `main` then calls
    /// `una_rt_http_enable`, the only path to the TLS stack (ADR 0074 H7, #1148).
    http: bool,
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
            hashing: false,
            threads: false,
            http: false,
        })
    }

    /// The function or predicate `path` of the package, declared now and compiled by
    /// [`Emitter::drain`].
    fn function(&mut self, path: &Path) -> Result<Callee, CodegenError> {
        self.function_entry(path, false)
    }

    /// Which parameters of the function `path` are owned (ADR 0072 D2), without declaring it:
    /// empty when every parameter is borrowed, or `path` is no function of the package.
    fn owned(&self, path: &Path) -> Vec<bool> {
        self.owned
            .get(&format!("{}.{}", path.module, path.name))
            .cloned()
            .unwrap_or_default()
    }

    /// An entry of the function or predicate `path`, declared now and compiled by
    /// [`Emitter::drain`]. With `borrowed`, the entry that borrows every parameter (#284): the
    /// same body, compiled as if no parameter were owned, for the calls that would retain
    /// every argument of an owned parameter. Each entry is compiled only when a call uses it,
    /// so a function has at most two.
    fn function_entry(&mut self, path: &Path, borrowed: bool) -> Result<Callee, CodegenError> {
        let qualified = format!("{}.{}", path.module, path.name);
        // A space never occurs in a qualified name, so the key of a borrowed entry is apart.
        let key = if borrowed {
            format!("{qualified} borrowed")
        } else {
            qualified.clone()
        };
        if let Some(found) = self.functions.get(&key) {
            return Ok(found.clone());
        }
        let body = callee(self.program, &path.module, &path.name).ok_or_else(|| {
            CodegenError::Unsupported(format!(
                "calls of `{qualified}`, which is not a function of the package"
            ))
        })??;
        let mut body = body;
        body.owned = if borrowed {
            Vec::new()
        } else {
            self.owned.get(&qualified).cloned().unwrap_or_default()
        };
        let name = format!("una_fn_{}", self.functions.len());
        let id = self.declare(&name, body.params.len() + body.uses.len())?;
        let callee = Callee {
            id,
            uses: body.uses.clone(),
            constraints: body.constraints.clone(),
        };
        self.functions.insert(key, callee.clone());
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
                // A lambda owns its parameters, as every call through a function value gives
                // them: its body moves them at their last use, or releases them.
                owned: vec![true; params.len()],
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
        let config = self.module.target_config();
        let (una_main_id, mut context) = self.una_main()?;
        {
            let mut builder_context = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
            let block = builder.create_block();
            builder.switch_to_block(block);
            builder.seal_block(block);
            debug_assert!(main.params.is_empty());
            // `una_main`: calls `main` of Una, whose result is `unit`, and returns 0, or reports
            // its failure (U6001) and returns 1.
            let instances = self.instances(&mut builder, main)?;
            let main_ref = self.module.declare_func_in_func(main_id, builder.func);
            let call = builder.ins().call(main_ref, &instances);
            let results = builder.inst_results(call).to_vec();
            self.release_all(&mut builder, &instances)?;
            let (Some(&value), Some(&failed)) = (results.first(), results.get(1)) else {
                return Err(internal("`main` lost its result"));
            };
            let returned = builder.create_block();
            let failing = Self::failing(&mut builder);
            builder
                .ins()
                .brif(failed, failing, &[value.into()], returned, &[]);
            builder.switch_to_block(returned);
            builder.seal_block(returned);
            let zero = builder.ins().iconst(types::I64, 0);
            builder.ins().return_(&[zero]);
            self.report_failure(&mut builder, failing)?;
            builder.finalize(config);
        }
        self.define(una_main_id, &mut context)?;
        self.c_main(una_main_id, meta)
    }

    /// The C `main` of a driven program (ADR 0116 rule 3), whose `una_main` runs the host's
    /// loop: `init`, whose failure is U6001; then, for each tick the runtime gives,
    /// `step(edit state, input: tick)`, whose Core result is `(state, unit)`; then, when the
    /// runtime gives no more ticks, `on_stop(state)` when the module declares it. The state
    /// stays one owned word from one step to the next, and each call gives its owned
    /// parameters away (ADR 0072 D2), the caller releasing the borrowed ones.
    fn drive_entry(
        &mut self,
        (init_id, init): (FuncId, &Fun),
        (step, step_id): (&Fun, FuncId),
        on_stop: Option<(&Fun, FuncId)>,
        owned: &Owned,
        meta: &str,
    ) -> Result<(), CodegenError> {
        let config = self.module.target_config();
        let (una_main_id, mut context) = self.una_main()?;
        {
            let mut builder_context = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
            let block = builder.create_block();
            builder.switch_to_block(block);
            builder.seal_block(block);
            let failing = Self::failing(&mut builder);

            // `init()`, with its instances.
            let instances = self.instances(&mut builder, init)?;
            let init_ref = self.module.declare_func_in_func(init_id, builder.func);
            let call = builder.ins().call(init_ref, &instances);
            let results = builder.inst_results(call).to_vec();
            self.release_all(&mut builder, &instances)?;
            let (Some(&state), Some(&failed)) = (results.first(), results.get(1)) else {
                return Err(internal("`init` lost its result"));
            };
            let started = builder.create_block();
            builder
                .ins()
                .brif(failed, failing, &[state.into()], started, &[]);
            builder.switch_to_block(started);
            builder.seal_block(started);
            let step_instances = self.instances(&mut builder, step)?;
            let head = builder.create_block();
            builder.append_block_param(head, types::I64);
            builder.ins().jump(head, &[state.into()]);

            // The head of the loop: the next tick, or 0 when the loop stops.
            builder.switch_to_block(head);
            let state = *builder
                .block_params(head)
                .first()
                .ok_or_else(|| internal("the loop lost its state"))?;
            let tick = self.runtime_call(&mut builder, "una_rt_drive_tick", &[])?;
            let stepping = builder.create_block();
            let done = builder.create_block();
            builder.ins().brif(tick, stepping, &[], done, &[]);

            // `step(edit state, input: tick)`.
            builder.switch_to_block(stepping);
            builder.seal_block(stepping);
            let step_ref = self.module.declare_func_in_func(step_id, builder.func);
            let mut args = vec![state, tick];
            args.extend(&step_instances);
            let call = builder.ins().call(step_ref, &args);
            let results = builder.inst_results(call).to_vec();
            for (position, &arg) in [state, tick].iter().enumerate() {
                if !owned.step.get(position).copied().unwrap_or(false) {
                    self.runtime_call(&mut builder, "una_rt_release", &[arg])?;
                }
            }
            let (Some(&stepped), Some(&failed)) = (results.first(), results.get(1)) else {
                return Err(internal("`step` lost its result"));
            };
            // `step` declares no `fails` (U3029), so it never fails; if it did, the run would
            // end as a failure of `init` does.
            let next = builder.create_block();
            builder
                .ins()
                .brif(failed, failing, &[stepped.into()], next, &[]);
            builder.switch_to_block(next);
            builder.seal_block(next);
            // The new state, the first component of the result, borrowed from it: retained,
            // then the result released.
            let zero = builder.ins().iconst(types::I64, 0);
            let updated = self.runtime_call(&mut builder, "una_rt_field", &[stepped, zero])?;
            self.runtime_call(&mut builder, "una_rt_retain", &[updated])?;
            self.runtime_call(&mut builder, "una_rt_release", &[stepped])?;
            builder.ins().jump(head, &[updated.into()]);
            builder.seal_block(head);

            // The end of the loop: `on_stop(state)`, then 0.
            builder.switch_to_block(done);
            builder.seal_block(done);
            self.release_all(&mut builder, &step_instances)?;
            match on_stop {
                Some((on_stop, on_stop_id)) => {
                    let instances = self.instances(&mut builder, on_stop)?;
                    let on_stop_ref = self.module.declare_func_in_func(on_stop_id, builder.func);
                    let mut args = vec![state];
                    args.extend(&instances);
                    let call = builder.ins().call(on_stop_ref, &args);
                    let results = builder.inst_results(call).to_vec();
                    if !owned.on_stop.first().copied().unwrap_or(false) {
                        self.runtime_call(&mut builder, "una_rt_release", &[state])?;
                    }
                    self.release_all(&mut builder, &instances)?;
                    let (Some(&value), Some(&failed)) = (results.first(), results.get(1)) else {
                        return Err(internal("`on_stop` lost its result"));
                    };
                    // `on_stop` declares no `fails` either (U3029).
                    let ended = builder.create_block();
                    builder
                        .ins()
                        .brif(failed, failing, &[value.into()], ended, &[]);
                    builder.switch_to_block(ended);
                    builder.seal_block(ended);
                }
                None => {
                    self.runtime_call(&mut builder, "una_rt_release", &[state])?;
                }
            }
            let zero = builder.ins().iconst(types::I64, 0);
            builder.ins().return_(&[zero]);
            self.report_failure(&mut builder, failing)?;
            builder.finalize(config);
        }
        self.define(una_main_id, &mut context)?;
        self.c_main(una_main_id, meta)
    }

    /// `una_main`, declared, with a context whose signature returns the exit status.
    fn una_main(&mut self) -> Result<(FuncId, Context), CodegenError> {
        let mut una_main = self.module.make_signature();
        una_main.returns.push(AbiParam::new(types::I64));
        let una_main_id = self
            .module
            .declare_function("una_main", Linkage::Local, &una_main)
            .map_err(internal)?;
        let mut context = self.module.make_context();
        context.func.signature = una_main;
        context.func.name = UserFuncName::user(0, una_main_id.as_u32());
        Ok((una_main_id, context))
    }

    /// A host instance for each capability `function` uses (`spec/11 §2`), which it takes after
    /// its parameters.
    fn instances(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        function: &Fun,
    ) -> Result<Vec<ir::Value>, CodegenError> {
        let mut instances = Vec::new();
        for need in &function.uses {
            let name = self.bytes(&need.capability)?;
            let name = self.module.declare_data_in_func(name, builder.func);
            let address = builder.ins().symbol_value(types::I64, name);
            let len = i64::try_from(need.capability.len()).map_err(internal)?;
            let len = builder.ins().iconst(types::I64, len);
            let text = self.runtime_call(builder, "una_rt_text_literal", &[address, len])?;
            let instance = self.runtime_call(builder, "una_rt_host_instance", &[text])?;
            instances.push(instance);
        }
        Ok(instances)
    }

    /// Releases each of `values`.
    fn release_all(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        values: &[ir::Value],
    ) -> Result<(), CodegenError> {
        for &value in values {
            self.runtime_call(builder, "una_rt_release", &[value])?;
        }
        Ok(())
    }

    /// A block of `una_main` that takes a failure, which [`Emitter::report_failure`] fills
    /// once every branch to it is emitted.
    fn failing(builder: &mut FunctionBuilder<'_>) -> ir::Block {
        let failing = builder.create_block();
        builder.append_block_param(failing, types::I64);
        failing
    }

    /// Fills and seals `failing`: it reports the failure it is given (U6001) and returns 1.
    fn report_failure(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        failing: ir::Block,
    ) -> Result<(), CodegenError> {
        builder.switch_to_block(failing);
        builder.seal_block(failing);
        let value = *builder
            .block_params(failing)
            .first()
            .ok_or_else(|| internal("the failure lost its value"))?;
        let report = *self
            .runtime
            .get("una_rt_main_failed")
            .ok_or_else(|| internal("the runtime lacks `una_rt_main_failed`"))?;
        let report = self.module.declare_func_in_func(report, builder.func);
        builder.ins().call(report, &[value]);
        let one = builder.ins().iconst(types::I64, 1);
        builder.ins().return_(&[one]);
        Ok(())
    }

    /// The C `main`, which calls `una_rt_start` of the runtime with the process arguments, the
    /// program's description and `una_main`.
    fn c_main(&mut self, una_main_id: FuncId, meta: &str) -> Result<(), CodegenError> {
        let pointer = self.module.target_config().pointer_type();
        let call_conv = self.module.target_config().default_call_conv;
        let config = self.module.target_config();
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
            if self.http {
                self.runtime_call(&mut builder, "una_rt_http_enable", &[])?;
            }
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

#[cfg(test)]
mod tests {
    use super::RUNTIME;

    /// Each runtime function that the generated code imports is defined in the runtime's FFI
    /// module (`crates/una-rt/src/ffi.rs`), in its table of entry points (`exported!`) or as a
    /// function of its own, and takes as many words as it gives it: a word more or less would
    /// link and then read the wrong words (#815).
    #[test]
    fn the_imported_runtime_functions_take_the_words_the_runtime_exports() {
        let ffi = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../una-rt/src/ffi.rs"),
        )
        .unwrap_or_default();
        // The parameters after `start`, up to the closing parenthesis: their count.
        let arity = |start: usize| {
            let rest = ffi.get(start..).unwrap_or_default();
            let params = rest.split(')').next().unwrap_or_default();
            params
                .split(',')
                .filter(|param| !param.trim().is_empty())
                .count()
        };
        for &(name, params, _) in RUNTIME {
            let entry = format!("\n    {name} => ");
            let function = format!("fn {name}(");
            let found = if let Some(at) = ffi.find(&entry) {
                let line = ffi.get(at..).unwrap_or_default();
                line.find(", (").map(|open| arity(at + open + 3))
            } else {
                ffi.find(&function).map(|at| arity(at + function.len()))
            };
            assert_eq!(found, Some(params), "{name}");
        }
    }
}
