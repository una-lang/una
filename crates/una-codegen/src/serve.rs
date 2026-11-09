//! The driver of a `serve` section in native code (ADR 0123 rule 16, `spec/31 §7` rule 4),
//! written into the executable as the loop of a driven program is (`Emitter::drive_entry`):
//! engine code equivalent to the program of rule 4, not Una source or Core IR that the package
//! gains (decision S6-D4).
//!
//! `una_main` hands the runtime's driver (`una_rt_serve_drive`) a closure of the compiled
//! `answer`, a lambda that the driver starts as a task per request with the request's exchange
//! as its one captured value. `answer` calls `handle` with the request and gives what it
//! returned to `una_rt_serve_answer`, which answers. When the driver's loop ends at a host stop
//! and its tasks drained, `una_main` calls `on_stop`.

use cranelift_codegen::ir::{InstBuilder, UserFuncName, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Module};
use una_core::{Base, Fun, Label, Level, Path, Program, Type};

use crate::{CodegenError, Described, Emitter, function, internal, isa};

/// The word of a stopped task (`lower::STOPPED`).
const STOPPED: i64 = -1;

/// The label of the driver's values: an `Exchange` and a `Reply` are `internal` (`spec/31 §1`
/// rule 2).
fn internal_label() -> Label {
    Label {
        level: Level::Internal,
        ..Label::public()
    }
}

/// The object file of a package whose `serve` section names `module` (ADR 0123 rule 16): its
/// `handle` and `on_stop`, the functions they reach, and the driver. `una build` checked the
/// functions' shapes; `handle`'s failures become `500`, and `on_stop` declares none (U3029),
/// so `meta`'s `failure` is `None`.
pub fn compile_serve(
    program: &Program,
    module: &str,
    meta: &dyn Fn(&Described<'_>) -> String,
) -> Result<Vec<u8>, CodegenError> {
    let handle = function(program, module, "handle").ok_or(CodegenError::NoMain)?;
    let at = |name: &str| Path {
        module: module.to_owned(),
        name: name.to_owned(),
    };
    let mut emitter = Emitter::new(program, isa()?)?;
    let handle_id = emitter.function(&at("handle"))?.id;
    let on_stop = match function(program, module, "on_stop") {
        Some(on_stop) => Some((on_stop, emitter.function(&at("on_stop"))?.id)),
        None => None,
    };
    emitter.drain()?;
    if emitter.types.unknown() {
        return Err(internal(
            "a type of the program holds a kind that has no declaration",
        ));
    }
    // The driver's tasks and its operations of `HttpServer`: the runtime starts its workers and
    // its serve threads before the driver (S4 P8).
    emitter.threads = true;
    emitter.serve = true;
    let next = emitter.server_op("next", &[])?;
    let reply = emitter.server_op("reply", &[internal_label(), internal_label()])?;
    let site = i64::try_from(emitter.sites.len()).map_err(internal)?;
    emitter.sites.push(handle.span);
    let owned = emitter.owned(&at("handle"));
    let answer = emitter.answer((handle_id, handle), &owned, reply, site)?;
    let meta = meta(&Described {
        sites: &emitter.sites,
        types: emitter.types.shapes(),
        ops: &emitter.ops,
        failure: None,
        hashing: emitter.hashing,
        threads: emitter.threads,
        frame_bound: emitter.frames.bytes(),
    });
    emitter.serve_entry(answer, on_stop, next, site, &meta)?;
    emitter
        .module
        .finish()
        .emit()
        .map_err(|error| internal(error.to_string()))
}

impl Emitter<'_> {
    /// The index of the event of the driver's `@server.<op>`, described as the operation's call
    /// in the program of rule 4 describes it: its arguments labeled `labels`, its outcome
    /// `internal`.
    fn server_op(&mut self, op: &str, labels: &[Label]) -> Result<i64, CodegenError> {
        let declared = self
            .capability(&Path {
                module: una_core::HOST_MODULE.to_owned(),
                name: "HttpServer".to_owned(),
            })
            .and_then(|ops| ops.iter().find(|o| o.name == op))
            .ok_or_else(|| internal(format!("`HttpServer` has no operation `{op}`")))?;
        let redaction =
            |label: &Label| (!label.is_public()).then(|| crate::describe::level(label.level));
        let args: Vec<serde_json::Value> = declared
            .params
            .iter()
            .zip(labels)
            .map(|((_, ty), label)| {
                serde_json::json!({ "type": self.types.of(ty), "redacted": redaction(label) })
            })
            .collect();
        let kind = declared.error.as_ref().ok_or_else(|| {
            internal(format!(
                "the error kind of `HttpServer.{op}` is not resolved"
            ))
        })?;
        let error = self.types.of(&Type::Base {
            base: Base::Named {
                module: kind.module.clone(),
                name: kind.name.clone(),
                args: Vec::new(),
            },
            refinements: Vec::new(),
            label: Label::public(),
        });
        let result = &declared.result;
        let index = i64::try_from(self.ops.len()).map_err(internal)?;
        let description = serde_json::json!({
            "capability": "HttpServer",
            "op": op,
            "params": declared.params.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>(),
            "args": args,
            "result": {
                "type": self.types.of(result),
                "redacted": redaction(&result.deep_label()),
            },
            "outcome": redaction(&internal_label().join(&result.deep_label())),
            "error": error,
        });
        self.ops.push(description);
        Ok(index)
    }

    /// `answer(exchange)` of the program of rule 4, the code of the closure that each started
    /// task runs: `handle(exchange.request)`, then `una_rt_serve_answer`, which answers with
    /// what it returned, `500` for a failure. The closure holds the exchange; `handle` takes
    /// the host instances of its `uses`, as `main` does.
    fn answer(
        &mut self,
        (handle_id, handle): (FuncId, &Fun),
        owned: &[bool],
        reply: i64,
        site: i64,
    ) -> Result<FuncId, CodegenError> {
        let id = self.declare("una_serve_answer", 1)?;
        let mut context = self.module.make_context();
        context.func.signature = self
            .module
            .declarations()
            .get_function_decl(id)
            .signature
            .clone();
        context.func.name = UserFuncName::user(0, id.as_u32());
        let config = self.module.target_config();
        {
            let mut builder_context = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
            let block = builder.create_block();
            builder.append_block_params_for_function_params(block);
            builder.switch_to_block(block);
            builder.seal_block(block);
            let closure = *builder
                .block_params(block)
                .first()
                .ok_or_else(|| internal("`answer` lost its closure"))?;
            let zero = builder.ins().iconst(types::I64, 0);
            let exchange = self.runtime_call(&mut builder, "una_rt_field", &[closure, zero])?;
            let request = self.runtime_call(&mut builder, "una_rt_field", &[exchange, zero])?;
            let gives = owned.first().copied().unwrap_or(false);
            if gives {
                self.runtime_call(&mut builder, "una_rt_retain", &[request])?;
            }
            let instances = self.instances(&mut builder, handle)?;
            // The call of `handle` counts toward the maximum depth, as every call does.
            let site_word = builder.ins().iconst(types::I64, site);
            let stopping = self.runtime_call(&mut builder, "una_rt_enter", &[site_word])?;
            let refused = builder.create_block();
            let calling = builder.create_block();
            builder.ins().brif(stopping, refused, &[], calling, &[]);
            builder.switch_to_block(refused);
            builder.seal_block(refused);
            self.release_all(&mut builder, &instances)?;
            if gives {
                self.runtime_call(&mut builder, "una_rt_release", &[request])?;
            }
            let stopped = builder.ins().iconst(types::I64, STOPPED);
            let two = builder.ins().iconst(types::I64, 2);
            builder.ins().return_(&[stopped, two]);
            builder.switch_to_block(calling);
            builder.seal_block(calling);
            let handle_ref = self.module.declare_func_in_func(handle_id, builder.func);
            let mut args = vec![request];
            args.extend(&instances);
            let call = builder.ins().call(handle_ref, &args);
            let results = builder.inst_results(call).to_vec();
            self.runtime_call(&mut builder, "una_rt_leave", &[])?;
            self.release_all(&mut builder, &instances)?;
            let (Some(&value), Some(&failed)) = (results.first(), results.get(1)) else {
                return Err(internal("`handle` lost its result"));
            };
            let reply = builder.ins().iconst(types::I64, reply);
            let status = self.runtime_call(
                &mut builder,
                "una_rt_serve_answer",
                &[reply, site_word, exchange, value, failed],
            )?;
            // `unit`, or the stop of the task.
            let stops = builder.ins().icmp_imm_s(IntCC::NotEqual, status, 0);
            let unit = builder.ins().iconst(types::I64, 0);
            let stop = stopped_word(&mut builder);
            let word = builder.ins().select(stops, stop, unit);
            builder.ins().return_(&[word, status]);
            builder.finalize(config);
        }
        self.define(id, &mut context)?;
        Ok(id)
    }

    /// The C `main` of a package with a `serve` section, whose `una_main` runs the driver: the
    /// requests until a host stop drains them, then `on_stop` when the module declares it.
    /// At the hard stop the driver stops, as the run's main task does, and `on_stop` does not
    /// run.
    fn serve_entry(
        &mut self,
        answer: FuncId,
        on_stop: Option<(&Fun, FuncId)>,
        next: i64,
        site: i64,
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
            let code = self.module.declare_func_in_func(answer, builder.func);
            let address = builder.ins().func_addr(types::I64, code);
            let arity = builder.ins().iconst(types::I64, 0);
            let template = self.runtime_call(&mut builder, "una_rt_closure", &[address, arity])?;
            let next = builder.ins().iconst(types::I64, next);
            let site = builder.ins().iconst(types::I64, site);
            let status =
                self.runtime_call(&mut builder, "una_rt_serve_drive", &[template, next, site])?;
            self.runtime_call(&mut builder, "una_rt_release", &[template])?;
            let drained = builder.create_block();
            let stopped = stopped_word(&mut builder);
            builder
                .ins()
                .brif(status, failing, &[stopped.into()], drained, &[]);
            builder.switch_to_block(drained);
            builder.seal_block(drained);
            if let Some((on_stop, on_stop_id)) = on_stop {
                let instances = self.instances(&mut builder, on_stop)?;
                let on_stop_ref = self.module.declare_func_in_func(on_stop_id, builder.func);
                let call = builder.ins().call(on_stop_ref, &instances);
                let results = builder.inst_results(call).to_vec();
                self.release_all(&mut builder, &instances)?;
                let (Some(&value), Some(&failed)) = (results.first(), results.get(1)) else {
                    return Err(internal("`on_stop` lost its result"));
                };
                // `on_stop` declares no `fails` (U3029); it stops at the hard stop only.
                let ended = builder.create_block();
                builder
                    .ins()
                    .brif(failed, failing, &[value.into()], ended, &[]);
                builder.switch_to_block(ended);
                builder.seal_block(ended);
            }
            let zero = builder.ins().iconst(types::I64, 0);
            builder.ins().return_(&[zero]);
            self.report_failure(&mut builder, failing)?;
            builder.finalize(config);
        }
        self.define(una_main_id, &mut context)?;
        self.c_main(una_main_id, meta)
    }
}

/// The word of a stopped task, in `builder`'s function.
fn stopped_word(builder: &mut FunctionBuilder<'_>) -> cranelift_codegen::ir::Value {
    builder.ins().iconst(types::I64, STOPPED)
}
