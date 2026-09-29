//! Elaboration to Core IR (`spec/09 §3`): the desugaring of the declarations that checked
//! without diagnostics, from what pass 2 resolved (DECISION D-098).

use std::collections::BTreeMap;

use una_core::{
    Arm as CoreArm, Atom, Base, Bound, CapOp as CoreCapOp, Ctor, DataVariant, Decl, Example,
    Expectation, Expr as CoreExpr, Fun, HandlerOp as CoreHandlerOp, Label, Module as CoreModule,
    Name, Path, Prim, Program, Rhs, Source, Type, Use,
};
use una_syntax::ast::{
    Arg, BinaryOp, Block, BlockExpr, Body, CapBindingValue, DeclKind, ElseBranch, ExampleDecl,
    ExpectationKind, Expr, ExprKind, Handled, HandlingKind, IfBlock, IsTarget, LetValue, Literal,
    PatternKind, Range, SignedInt, Stmt, TextLit, TextPart, UnaryOp, VariantExpr, WhenExpr,
};
use una_syntax::{SourceDb, Span};

use super::Findings;
use super::env::{Entity, Env, FnAst, FnKind, Need};
use super::records::{Records, Res};
use super::ty::{Declared, FnUse, Refinement, Ty};

/// One binding of a `let` chain being built.
type Let = (Name, Type, Bound);

/// Elaborates the package's modules, in path order; a declaration with a diagnostic, or with
/// a resolution missing, is left out.
pub(crate) fn program(env: &Env<'_>, findings: &Findings, sources: &SourceDb) -> Program {
    let mut modules: Vec<CoreModule> = (0..env.prelude)
        .filter(|module| env.reports(*module))
        .map(|module| elaborate_module(env, findings, sources, module))
        .collect();
    modules.sort_by(|a, b| a.path.cmp(&b.path));
    Program { modules }
}

fn elaborate_module(
    env: &Env<'_>,
    findings: &Findings,
    sources: &SourceDb,
    module: usize,
) -> CoreModule {
    let mut out = CoreModule {
        path: env
            .modules
            .get(module)
            .map(|m| m.path.clone())
            .unwrap_or_default(),
        ..CoreModule::default()
    };
    let Some(m) = env.modules.get(module) else {
        return out;
    };
    for (decl, entity) in m.tree.decls.iter().zip(m.decls.clone()) {
        if findings.failed.contains(&decl.span) {
            continue;
        }
        let mut fx = Fx::new(env, &findings.records, sources, module);
        match (&decl.kind, entity) {
            (DeclKind::Example(example), _) => {
                if let Some(example) = fx.example(example) {
                    out.examples.push(example);
                }
            }
            (_, Some(entity)) => {
                if let Some(core) = fx.decl(&decl.kind, entity) {
                    out.decls.push(core);
                }
            }
            _ => {}
        }
    }
    out
}

/// The elaboration of one declaration.
struct Fx<'e, 'a> {
    env: &'e Env<'a>,
    records: &'e Records,
    sources: &'e SourceDb,
    module: usize,
    type_params: Vec<&'a str>,
    /// Source names in scope and their binders, innermost last.
    scope: Vec<(String, Name)>,
    /// How many binders each source name has had, for unique names (`spec/09 §1` rule 3).
    used: BTreeMap<String, usize>,
    temps: usize,
    /// The binders of `error`, `it` and `result` where they are allowed.
    error: Option<Name>,
    it: Option<Name>,
    result: Option<Name>,
    /// A resolution was missing: the declaration is not elaborated.
    broken: bool,
}

impl<'e, 'a> Fx<'e, 'a> {
    fn new(env: &'e Env<'a>, records: &'e Records, sources: &'e SourceDb, module: usize) -> Self {
        Fx {
            env,
            records,
            sources,
            module,
            type_params: Vec::new(),
            scope: Vec::new(),
            used: BTreeMap::new(),
            temps: 0,
            error: None,
            it: None,
            result: None,
            broken: false,
        }
    }

    // ----- names -----

    fn name(&self, ident: &una_syntax::ast::Ident) -> &'a str {
        self.env.text(self.module, ident.name)
    }

    /// A unique binder for source name `name`, in scope until the scope is cut back.
    fn bind(&mut self, name: &str) -> Name {
        let count = self.used.entry(name.to_owned()).or_insert(0);
        *count += 1;
        let binder = if *count == 1 {
            name.to_owned()
        } else {
            format!("{name}%{count}")
        };
        self.scope.push((name.to_owned(), binder.clone()));
        binder
    }

    /// A fresh temporary, `%t<n>`.
    fn temp(&mut self) -> Name {
        let name = format!("%t{}", self.temps);
        self.temps += 1;
        name
    }

    fn local(&self, name: &str) -> Option<Name> {
        self.scope
            .iter()
            .rev()
            .find(|(source, _)| source == name)
            .map(|(_, binder)| binder.clone())
    }

    // ----- types -----

    fn ty(&self, ty: &Ty) -> Type {
        match ty {
            Ty::Labeled(label, inner) => with_label(self.ty(inner), label),
            Ty::Fn(function) => Type::Fn {
                params: function.params.iter().map(|p| self.ty(p)).collect(),
                result: Box::new(self.ty(&function.result)),
                uses: self.uses_of(&function.uses),
                fails: function.fails.as_ref().map(|f| self.kind_name(f)),
                label: Label::public(),
            },
            other => Type::Base {
                base: self.base(other),
                refinements: Vec::new(),
                label: Label::public(),
            },
        }
    }

    fn base(&self, ty: &Ty) -> Base {
        match ty {
            Ty::Int => Base::Int,
            Ty::Bool => Base::Bool,
            Ty::Text => Base::Text,
            Ty::Unit => Base::Unit,
            Ty::Hashed => Base::Hashed,
            Ty::List(element) => Base::Named {
                name: "List".to_owned(),
                args: vec![self.ty(element)],
            },
            Ty::Kind(id, args) => Base::Named {
                name: self.env.kind_name(*id).to_owned(),
                args: args.iter().map(|a| self.ty(a)).collect(),
            },
            Ty::Param(index) => Base::Param(
                self.type_params
                    .get(*index)
                    .map_or_else(|| "?".to_owned(), |p| (*p).to_owned()),
            ),
            Ty::Labeled(_, inner) => self.base(inner),
            Ty::Fn(_) | Ty::Var(_) | Ty::Never | Ty::Error => Base::Unknown,
        }
    }

    /// A declared type, with the names of its refinements.
    fn declared(&self, declared: &Declared) -> Type {
        let mut ty = self.ty(&declared.base);
        if let Type::Base { refinements, .. } = &mut ty {
            for refinement in &declared.refinements {
                refinements.push(match refinement {
                    Refinement::Alias(id, _) => self
                        .env
                        .aliases
                        .get(*id)
                        .map_or_else(|| "?".to_owned(), |a| a.name.to_owned()),
                    Refinement::Range(text) => text.clone(),
                });
            }
        }
        ty
    }

    fn kind_name(&self, ty: &Ty) -> String {
        match ty {
            Ty::Kind(id, _) => self.env.kind_name(*id).to_owned(),
            Ty::Labeled(_, inner) => self.kind_name(inner),
            _ => "?".to_owned(),
        }
    }

    fn uses_of(&self, uses: &[FnUse]) -> Vec<Use> {
        uses.iter()
            .map(|u| Use {
                cap: u.name.clone(),
                capability: u
                    .capability
                    .and_then(|c| self.env.capabilities.get(c))
                    .map_or_else(|| "?".to_owned(), |c| c.name.to_owned()),
                ops: u.ops.clone(),
            })
            .collect()
    }

    fn needs(&self, needs: &[Need]) -> Vec<Use> {
        let uses: Vec<FnUse> = needs.iter().map(|n| n.usage.clone()).collect();
        self.uses_of(&uses)
    }

    /// The recorded type of the expression or binder at `span`.
    fn type_at(&self, span: Span) -> Type {
        self.records
            .types
            .get(&span)
            .map_or_else(unknown, |ty| self.ty(ty))
    }

    fn path(&self, module: usize, name: &str) -> Path {
        Path {
            module: self
                .env
                .modules
                .get(module)
                .map(|m| m.path.clone())
                .unwrap_or_default(),
            name: name.to_owned(),
        }
    }

    fn function_path(&self, id: usize) -> Path {
        match self.env.functions.get(id) {
            Some(f) => self.path(f.module, f.name),
            None => self.path(self.module, "?"),
        }
    }

    fn ctor(&mut self, kind: usize, variant: Option<usize>) -> Ctor {
        let Some(def) = self.env.kinds.get(kind) else {
            self.broken = true;
            return Ctor {
                kind: self.path(self.module, "?"),
                name: "?".to_owned(),
                index: 0,
            };
        };
        let name = match variant {
            Some(index) => def.variants.get(index).map_or("?", |v| v.name),
            None => def.name,
        };
        Ctor {
            kind: self.path(def.module, def.name),
            name: name.to_owned(),
            index: variant.unwrap_or(0),
        }
    }

    fn res(&mut self, span: Span) -> Option<Res> {
        let res = self.records.res.get(&span).cloned();
        if res.is_none() {
            self.broken = true;
        }
        res
    }

    fn order(&self, span: Span) -> Vec<Option<usize>> {
        self.records.args.get(&span).cloned().unwrap_or_default()
    }

    // ----- declarations -----

    fn decl(&mut self, kind: &'a DeclKind, entity: Entity) -> Option<Decl> {
        let decl = match (kind, entity) {
            (DeclKind::Kind(_), Entity::Alias(id)) => {
                let alias = self.env.aliases.get(id)?;
                self.type_params = alias.params.clone();
                Decl::Alias {
                    name: alias.name.to_owned(),
                    params: alias.params.iter().map(|p| (*p).to_owned()).collect(),
                    target: self.declared(&alias.base),
                }
            }
            (DeclKind::Kind(_), Entity::Kind(id)) => {
                let def = self.env.kinds.get(id)?;
                self.type_params = def.params.clone();
                let field = |fx: &Self, f: &super::env::FieldDef<'_>| {
                    (f.name.to_owned(), fx.declared(&f.ty))
                };
                let (variants, record) = match &def.record {
                    Some(fields) => (
                        vec![DataVariant {
                            name: def.name.to_owned(),
                            fields: fields.iter().map(|f| field(self, f)).collect(),
                        }],
                        true,
                    ),
                    None => (
                        def.variants
                            .iter()
                            .map(|v| DataVariant {
                                name: v.name.to_owned(),
                                fields: v.fields.iter().map(|f| field(self, f)).collect(),
                            })
                            .collect(),
                        false,
                    ),
                };
                Decl::Data {
                    name: def.name.to_owned(),
                    params: def.params.iter().map(|p| (*p).to_owned()).collect(),
                    variants,
                    record,
                }
            }
            (DeclKind::Function(function), Entity::Function(id)) => {
                let def = self.env.functions.get(id)?;
                self.type_params = def.type_params.clone();
                let params = self.params(&def.params);
                let body = self.body(&function.body);
                Decl::Fun(Fun {
                    name: def.name.to_owned(),
                    type_params: def.type_params.iter().map(|p| (*p).to_owned()).collect(),
                    params,
                    result: self.declared(&def.result),
                    uses: self.needs(&def.uses),
                    fails: def.fails.as_ref().map(|f| self.kind_name(f)),
                    body,
                    span: function.name.span,
                })
            }
            (DeclKind::Predicate(predicate), Entity::Function(id)) => {
                let def = self.env.functions.get(id)?;
                let params = self.params(&def.params);
                let body = self.body(&predicate.body);
                Decl::Pred {
                    name: def.name.to_owned(),
                    reflect: matches!(def.kind, FnKind::Predicate { reflect: true }),
                    params,
                    body,
                }
            }
            (DeclKind::Capability(_), Entity::Capability(id)) => {
                let def = self.env.capabilities.get(id)?;
                Decl::Cap {
                    name: def.name.to_owned(),
                    ops: def
                        .ops
                        .iter()
                        .map(|op| CoreCapOp {
                            name: op.name.to_owned(),
                            params: op
                                .params
                                .iter()
                                .map(|p| (p.name.to_owned(), self.declared(&p.ty)))
                                .collect(),
                            result: self.declared(&op.result),
                            fails: op.fails.as_ref().map(|f| self.kind_name(f)),
                            accepts: op.accepts.clone(),
                        })
                        .collect(),
                }
            }
            (DeclKind::Handler(handler), Entity::Handler(id)) => {
                let def = self.env.handlers.get(id)?;
                let params = self.params(&def.params);
                let capability = def.capability.and_then(|c| self.env.capabilities.get(c));
                let mut ops = Vec::new();
                for op in &handler.ops {
                    let mark = self.scope.len();
                    let op_params: Vec<Name> = op
                        .params
                        .iter()
                        .map(|p| {
                            let name = self.name(p);
                            self.bind(name)
                        })
                        .collect();
                    let body = self.body(&op.body);
                    self.scope.truncate(mark);
                    ops.push(CoreHandlerOp {
                        name: self.name(&op.name).to_owned(),
                        params: op_params,
                        body,
                    });
                }
                Decl::Handler {
                    name: self.name(&handler.name).to_owned(),
                    params,
                    capability: capability.map_or_else(|| "?".to_owned(), |c| c.name.to_owned()),
                    uses: self.needs(&def.uses),
                    ops,
                }
            }
            (DeclKind::Policy(policy), Entity::Function(id)) => {
                let def = self.env.functions.get(id)?;
                let via_name = self.name(&policy.via);
                let via = match self.env.lower(self.module, via_name) {
                    Some(Entity::Function(via)) => self.function_path(via),
                    _ => return None,
                };
                Decl::Policy {
                    name: def.name.to_owned(),
                    from: def
                        .params
                        .first()
                        .map_or_else(unknown, |p| self.declared(&p.ty)),
                    to: self.declared(&def.result),
                    via,
                }
            }
            _ => return None,
        };
        (!self.broken).then_some(decl)
    }

    fn params(&mut self, params: &[super::env::ParamDef<'a>]) -> Vec<(Name, Type)> {
        params
            .iter()
            .map(|p| {
                let ty = self.declared(&p.ty);
                (self.bind(p.name), ty)
            })
            .collect()
    }

    fn example(&mut self, example: &'a ExampleDecl) -> Option<Example> {
        let name: String = example
            .name
            .parts
            .iter()
            .map(|part| match part {
                TextPart::Text(text) => text.clone(),
                TextPart::Interpolation(_) => String::new(),
            })
            .collect();
        let mut lets = Vec::new();
        let mut givens = Vec::new();
        for given in &example.givens {
            let handler = match self.res(given.handler.span) {
                Some(Res::Handler { id }) => id,
                _ => return None,
            };
            let args = self.call_args(&mut lets, given.handler.span, &given.handler.args);
            let handler_name = self.name(&given.handler.name);
            let module = self
                .env
                .handlers
                .get(handler)
                .map_or(self.module, |h| h.module);
            givens.push((
                self.name(&given.cap).to_owned(),
                Source::Handler {
                    handler: self.path(module, handler_name),
                    args,
                    given: true,
                },
            ));
        }
        let mut body = self.expr(&example.action);
        for (cap, source) in givens.into_iter().rev() {
            body = CoreExpr::With {
                cap,
                source,
                body: Box::new(body),
            };
        }
        let body = close(lets, body);
        let result = "result".to_owned();
        let mut expectations = Vec::new();
        for expectation in &example.expectations {
            let text = self.text(expectation.span);
            let line = self.sources.location(expectation.span).start.line;
            expectations.push(match &expectation.kind {
                ExpectationKind::Holds(expr) => {
                    self.result = Some(result.clone());
                    let check = self.expr(expr);
                    self.result = None;
                    Expectation::Holds {
                        result: result.clone(),
                        check,
                        text,
                        line,
                    }
                }
                ExpectationKind::Fails(variant) => Expectation::Fails {
                    variant: self.name(variant).to_owned(),
                    text,
                    line,
                },
                ExpectationKind::Performed { negated, cap, op } => Expectation::Performed {
                    cap: self.name(cap).to_owned(),
                    op: self.name(op).to_owned(),
                    negated: *negated,
                    text,
                    line,
                },
            });
        }
        (!self.broken).then_some(Example {
            name,
            body,
            expectations,
        })
    }

    fn text(&self, span: Span) -> String {
        self.sources
            .file(span.file)
            .text()
            .get(span.start..span.end)
            .unwrap_or("")
            .to_owned()
    }

    // ----- bodies and statements -----

    fn body(&mut self, body: &Body) -> CoreExpr {
        match body {
            Body::Line(expr) => self.expr(expr),
            Body::Block(block) => self.block(block),
        }
    }

    fn block(&mut self, block: &Block) -> CoreExpr {
        let mark = self.scope.len();
        let mut lets: Vec<Let> = Vec::new();
        let last = block.stmts.len().saturating_sub(1);
        let mut tail = CoreExpr::Atom(Atom::Unit);
        for (index, stmt) in block.stmts.iter().enumerate() {
            match stmt {
                Stmt::Let(stmt) => {
                    let value = match &stmt.value {
                        LetValue::Handled(handled) => self.handled(handled),
                        LetValue::BlockExpr(expr) => self.block_expr(expr),
                    };
                    let ty = self.type_at(stmt.name.span);
                    let name = self.name(&stmt.name);
                    let binder = self.bind(name);
                    bind_expr(&mut lets, binder, ty, value);
                }
                Stmt::Handled(handled) if index == last => tail = self.handled(handled),
                Stmt::BlockExpr(expr) if index == last => tail = self.block_expr(expr),
                Stmt::Handled(handled) => {
                    let value = self.handled(handled);
                    bind_expr(&mut lets, "_".to_owned(), unit(), value);
                }
                Stmt::BlockExpr(expr) => {
                    let value = self.block_expr(expr);
                    bind_expr(&mut lets, "_".to_owned(), unit(), value);
                }
            }
        }
        self.scope.truncate(mark);
        close(lets, tail)
    }

    /// A statement's expression with its `with` bindings and handling clauses (`spec/09 §3`).
    fn handled(&mut self, handled: &Handled) -> CoreExpr {
        let mut lets: Vec<Let> = Vec::new();
        let mut bindings = Vec::new();
        for binding in &handled.with {
            let cap = self.name(&binding.cap).to_owned();
            let source = match &binding.value {
                CapBindingValue::Cap(other) => Source::Rebind(self.name(other).to_owned()),
                CapBindingValue::Handler(call) => {
                    let handler = match self.res(call.span) {
                        Some(Res::Handler { id }) => id,
                        _ => return CoreExpr::Atom(Atom::Unit),
                    };
                    let args = self.call_args(&mut lets, call.span, &call.args);
                    let module = self
                        .env
                        .handlers
                        .get(handler)
                        .map_or(self.module, |h| h.module);
                    Source::Handler {
                        handler: self.path(module, self.name(&call.name)),
                        args,
                        given: false,
                    }
                }
            };
            bindings.push((cap, source));
        }
        let mut current = self.expr(&handled.expr);
        let value_ty = self.type_at(handled.expr.span);
        for handling in &handled.handlings {
            current = match &handling.kind {
                HandlingKind::ErrorPass => current,
                HandlingKind::ErrorFail(variant) => {
                    let error = self.bind("error");
                    self.error = Some(error.clone());
                    let handler = self.fail(variant);
                    self.error = None;
                    self.scope.pop();
                    CoreExpr::Handle {
                        body: Box::new(current),
                        error,
                        handler: Box::new(handler),
                    }
                }
                HandlingKind::ErrorUse(value) => {
                    let error = self.bind("error");
                    self.error = Some(error.clone());
                    let handler = self.expr(value);
                    self.error = None;
                    self.scope.pop();
                    CoreExpr::Handle {
                        body: Box::new(current),
                        error,
                        handler: Box::new(handler),
                    }
                }
                HandlingKind::NoneFail(variant) => {
                    let otherwise = self.fail(variant);
                    self.unwrap_option(current, value_ty.clone(), otherwise)
                }
                HandlingKind::NoneUse(value) => {
                    let otherwise = self.expr(value);
                    self.unwrap_option(current, value_ty.clone(), otherwise)
                }
            };
        }
        for (cap, source) in bindings.into_iter().rev() {
            current = CoreExpr::With {
                cap,
                source,
                body: Box::new(current),
            };
        }
        close(lets, current)
    }

    /// `let %t = option in case %t of { Some(v) ⇒ v } else ⇒ otherwise`.
    fn unwrap_option(&mut self, option: CoreExpr, ty: Type, otherwise: CoreExpr) -> CoreExpr {
        let Some(kind) = self.env.prelude_kind("Option") else {
            self.broken = true;
            return option;
        };
        let t = self.temp();
        let v = self.temp();
        let mut lets = Vec::new();
        bind_expr(&mut lets, t.clone(), ty, option);
        let some = self.ctor(kind, Some(0));
        close(
            lets,
            CoreExpr::Case {
                scrutinee: Atom::Var(t),
                arms: vec![CoreArm {
                    ctor: some,
                    binds: vec![Some(v.clone())],
                    body: CoreExpr::Atom(Atom::Var(v)),
                }],
                otherwise: Some(Box::new(otherwise)),
            },
        )
    }

    fn fail(&mut self, variant: &VariantExpr) -> CoreExpr {
        let mut lets = Vec::new();
        let (kind, index) = match self.res(variant.span) {
            Some(Res::Ctor { kind, variant }) => (kind, variant),
            _ => return CoreExpr::Atom(Atom::Unit),
        };
        let args = self.call_args(&mut lets, variant.span, &variant.args);
        let ctor = self.ctor(kind, index);
        close(
            lets,
            CoreExpr::Fail {
                ctor,
                args,
                span: variant.span,
            },
        )
    }

    fn block_expr(&mut self, expr: &BlockExpr) -> CoreExpr {
        match expr {
            BlockExpr::If(block) => self.if_block(block),
            BlockExpr::When(when) => self.when(when),
        }
    }

    fn if_block(&mut self, block: &IfBlock) -> CoreExpr {
        let mut lets = Vec::new();
        let cond = self.atom(&mut lets, &block.condition);
        let then = self.block(&block.then_block);
        let otherwise = match block.otherwise.as_ref() {
            ElseBranch::Block(otherwise) => self.block(otherwise),
            ElseBranch::If(next) => self.if_block(next),
        };
        close(
            lets,
            CoreExpr::If {
                cond,
                then: Box::new(then),
                otherwise: Box::new(otherwise),
            },
        )
    }

    /// `when`: `case` for variant arms, `if` chains for literal and range arms, `else` as the
    /// final branch. Without `else`, the last literal or range arm is the final branch: the
    /// arms are exhaustive, an obligation of `spec/12 §4`.
    fn when(&mut self, when: &WhenExpr) -> CoreExpr {
        let mut lets = Vec::new();
        let scrutinee = self.atom(&mut lets, &when.scrutinee);
        let variant_arms = when
            .arms
            .iter()
            .any(|arm| matches!(arm.pattern.kind, PatternKind::Variant { .. }));
        if variant_arms {
            let mut arms = Vec::new();
            let mut otherwise = None;
            for arm in &when.arms {
                match &arm.pattern.kind {
                    PatternKind::Variant { binds, .. } => {
                        let (kind, index) = match self.res(arm.pattern.span) {
                            Some(Res::Ctor { kind, variant }) => (kind, variant),
                            _ => continue,
                        };
                        let ctor = self.ctor(kind, index);
                        let fields = self
                            .env
                            .kinds
                            .get(kind)
                            .and_then(|k| index.and_then(|i| k.variants.get(i)))
                            .map_or(0, |v| v.fields.len());
                        let mark = self.scope.len();
                        let mut slots: Vec<Option<Name>> = vec![None; fields];
                        let order = self.order(arm.pattern.span);
                        for (bind, field) in binds.iter().zip(order) {
                            let local = bind.local.as_ref().unwrap_or(&bind.field);
                            let binder = self.bind(self.name(local));
                            if let Some(slot) = field.and_then(|f| slots.get_mut(f)) {
                                *slot = Some(binder);
                            }
                        }
                        let body = self.body(&arm.body);
                        self.scope.truncate(mark);
                        arms.push(CoreArm {
                            ctor,
                            binds: slots,
                            body,
                        });
                    }
                    PatternKind::Else => otherwise = Some(Box::new(self.body(&arm.body))),
                    PatternKind::Literal(_) | PatternKind::Range(_) => {}
                }
            }
            return close(
                lets,
                CoreExpr::Case {
                    scrutinee,
                    arms,
                    otherwise,
                },
            );
        }
        // Literal and range arms, from the last to the first.
        let mut chain: Option<CoreExpr> = None;
        for arm in when.arms.iter().rev() {
            let body = self.body(&arm.body);
            let test = match &arm.pattern.kind {
                PatternKind::Literal(literal) => {
                    let mut test_lets = Vec::new();
                    let value = self.literal_atom(&mut test_lets, literal, arm.pattern.span);
                    let t = self.temp();
                    test_lets.push((
                        t.clone(),
                        bool_ty(),
                        Bound::Rhs(Rhs::Prim {
                            prim: Prim::Eq,
                            args: vec![scrutinee.clone(), value],
                        }),
                    ));
                    Some((test_lets, Atom::Var(t)))
                }
                PatternKind::Range(range) => {
                    let mut test_lets = Vec::new();
                    let t = self.range_test(&mut test_lets, &scrutinee, range);
                    Some((test_lets, t))
                }
                PatternKind::Else | PatternKind::Variant { .. } => None,
            };
            chain = Some(match (chain, test) {
                (None, _) | (Some(_), None) => body,
                (Some(rest), Some((test_lets, cond))) => close(
                    test_lets,
                    CoreExpr::If {
                        cond,
                        then: Box::new(body),
                        otherwise: Box::new(rest),
                    },
                ),
            });
        }
        close(lets, chain.unwrap_or(CoreExpr::Atom(Atom::Unit)))
    }

    /// `lo <= x and x <= hi`, or `x < hi` for `..<`, as nested `if`s.
    fn range_test(&mut self, lets: &mut Vec<Let>, value: &Atom, range: &Range) -> Atom {
        let low = Atom::Int(signed(&range.low));
        let high = Atom::Int(signed(&range.high));
        let above = self.temp();
        lets.push((
            above.clone(),
            bool_ty(),
            Bound::Rhs(Rhs::Prim {
                prim: Prim::Le,
                args: vec![low, value.clone()],
            }),
        ));
        let below = self.temp();
        let prim = if range.exclusive { Prim::Lt } else { Prim::Le };
        let below_expr = CoreExpr::Let {
            name: below.clone(),
            ty: bool_ty(),
            value: Box::new(Bound::Rhs(Rhs::Prim {
                prim,
                args: vec![value.clone(), high],
            })),
            body: Box::new(CoreExpr::Atom(Atom::Var(below))),
        };
        let t = self.temp();
        lets.push((
            t.clone(),
            bool_ty(),
            Bound::Expr(CoreExpr::If {
                cond: Atom::Var(above),
                then: Box::new(below_expr),
                otherwise: Box::new(CoreExpr::Atom(Atom::Bool(false))),
            }),
        ));
        Atom::Var(t)
    }

    // ----- expressions -----

    /// An expression in its own right: a return path, a branch or a statement.
    fn expr(&mut self, expr: &Expr) -> CoreExpr {
        match &expr.kind {
            ExprKind::Paren(inner) => self.expr(inner),
            ExprKind::IfLine {
                condition,
                then,
                otherwise,
            } => {
                let mut lets = Vec::new();
                let cond = self.atom(&mut lets, condition);
                let then = self.expr(then);
                let otherwise = self.expr(otherwise);
                close(
                    lets,
                    CoreExpr::If {
                        cond,
                        then: Box::new(then),
                        otherwise: Box::new(otherwise),
                    },
                )
            }
            ExprKind::Fail(variant) => self.fail(variant),
            ExprKind::Attempt(operand) => CoreExpr::Attempt(Box::new(self.expr(operand))),
            _ => {
                let mut lets = Vec::new();
                let atom = self.atom(&mut lets, expr);
                close(lets, CoreExpr::Atom(atom))
            }
        }
    }

    /// Binds `rhs` to a fresh temporary of the type recorded at `span`.
    fn push(&mut self, lets: &mut Vec<Let>, span: Span, rhs: Rhs) -> Atom {
        let t = self.temp();
        let ty = self.type_at(span);
        lets.push((t.clone(), ty, Bound::Rhs(rhs)));
        Atom::Var(t)
    }

    /// Binds an expression to a fresh temporary of the type recorded at `span`.
    fn push_expr(&mut self, lets: &mut Vec<Let>, span: Span, expr: CoreExpr) -> Atom {
        let t = self.temp();
        let ty = self.type_at(span);
        bind_expr(lets, t.clone(), ty, expr);
        Atom::Var(t)
    }

    /// An expression as an atom, its intermediate results bound in `lets` (A-normal form).
    fn atom(&mut self, lets: &mut Vec<Let>, expr: &Expr) -> Atom {
        let span = expr.span;
        match &expr.kind {
            ExprKind::Literal(literal) => self.literal_atom(lets, literal, span),
            ExprKind::Paren(inner) => self.atom(lets, inner),
            ExprKind::Name(ident) => {
                let name = self.name(ident);
                if let Some(local) = self.local(name) {
                    return Atom::Var(local);
                }
                match self.res(ident.span) {
                    Some(Res::Function { id, type_args }) => {
                        let lambda = self.function_value(id, &type_args, ident.span);
                        self.push(lets, span, lambda)
                    }
                    _ => Atom::Unit,
                }
            }
            ExprKind::UpperName(ident) => match self.res(ident.span) {
                Some(Res::Ctor { kind, variant }) => {
                    let ctor = self.ctor(kind, variant);
                    self.push(
                        lets,
                        span,
                        Rhs::Construct {
                            ctor,
                            args: Vec::new(),
                        },
                    )
                }
                _ => Atom::Unit,
            },
            ExprKind::List(items) => {
                let items: Vec<Atom> = items.iter().map(|item| self.atom(lets, item)).collect();
                self.push(lets, span, Rhs::List(items))
            }
            ExprKind::Binary { op, left, right } => self.binary(lets, *op, left, right, span),
            ExprKind::Unary { op, operand } => {
                let operand = self.atom(lets, operand);
                let prim = match op {
                    UnaryOp::Neg => Prim::Neg,
                    UnaryOp::Not => Prim::Not,
                };
                self.push(
                    lets,
                    span,
                    Rhs::Prim {
                        prim,
                        args: vec![operand],
                    },
                )
            }
            ExprKind::Is { value, target } => {
                let value = self.atom(lets, value);
                match target {
                    IsTarget::Variant(ident) => match self.res(ident.span) {
                        Some(Res::Ctor { kind, variant }) => {
                            let fields = self
                                .env
                                .kinds
                                .get(kind)
                                .and_then(|k| variant.and_then(|i| k.variants.get(i)))
                                .map_or(0, |v| v.fields.len());
                            let ctor = self.ctor(kind, variant);
                            let test = CoreExpr::Case {
                                scrutinee: value,
                                arms: vec![CoreArm {
                                    ctor,
                                    binds: vec![None; fields],
                                    body: CoreExpr::Atom(Atom::Bool(true)),
                                }],
                                otherwise: Some(Box::new(CoreExpr::Atom(Atom::Bool(false)))),
                            };
                            self.push_expr(lets, span, test)
                        }
                        _ => Atom::Unit,
                    },
                    IsTarget::Literal(literal) => {
                        let literal = self.literal_atom(lets, literal, span);
                        self.push(
                            lets,
                            span,
                            Rhs::Prim {
                                prim: Prim::Eq,
                                args: vec![value, literal],
                            },
                        )
                    }
                }
            }
            ExprKind::In { value, range } => {
                let value = self.atom(lets, value);
                self.range_test(lets, &value, range)
            }
            ExprKind::IfLine { .. } | ExprKind::Fail(_) | ExprKind::Attempt(_) => {
                let inner = self.expr(expr);
                self.push_expr(lets, span, inner)
            }
            ExprKind::Lambda { params, body } => {
                let mark = self.scope.len();
                let (error, it, result) = (self.error.take(), self.it.take(), self.result.take());
                let params: Vec<(Name, Type)> = params
                    .iter()
                    .map(|p| {
                        let ty = self.type_at(p.name.span);
                        (self.bind(self.name(&p.name)), ty)
                    })
                    .collect();
                let body = self.expr(body);
                (self.error, self.it, self.result) = (error, it, result);
                self.scope.truncate(mark);
                self.push(
                    lets,
                    span,
                    Rhs::Lambda {
                        params,
                        body: Box::new(body),
                    },
                )
            }
            ExprKind::Checked {
                value,
                predicate,
                failure,
            } => {
                let value = self.atom(lets, value);
                let it = self.bind("it");
                let saved = self.it.replace(it.clone());
                let predicate = self.expr(predicate);
                self.it = saved;
                self.scope.pop();
                let (kind, index) = match self.res(failure.span) {
                    Some(Res::Ctor { kind, variant }) => (kind, variant),
                    _ => return Atom::Unit,
                };
                let args = self.call_args(lets, failure.span, &failure.args);
                let ctor = self.ctor(kind, index);
                self.push(
                    lets,
                    span,
                    Rhs::Checked {
                        value,
                        it,
                        predicate: Box::new(predicate),
                        ctor,
                        args,
                        span,
                    },
                )
            }
            ExprKind::Field { target, .. } => match self.res(span) {
                Some(Res::Len) => {
                    let value = self.atom(lets, target);
                    self.push(
                        lets,
                        span,
                        Rhs::Prim {
                            prim: Prim::Len,
                            args: vec![value],
                        },
                    )
                }
                Some(Res::Field { kind, index }) => {
                    let value = self.atom(lets, target);
                    let field = self
                        .env
                        .kinds
                        .get(kind)
                        .and_then(|k| k.record.as_ref())
                        .and_then(|fields| fields.get(index))
                        .map_or("?", |f| f.name)
                        .to_owned();
                    self.push(
                        lets,
                        span,
                        Rhs::Field {
                            value,
                            field,
                            index,
                        },
                    )
                }
                Some(Res::Function { id, type_args }) => {
                    let receiver = self.atom(lets, target);
                    self.call_function(lets, span, id, &type_args, Some(receiver), &[])
                }
                _ => Atom::Unit,
            },
            ExprKind::MethodCall { target, args, .. } => match self.res(span) {
                Some(Res::FieldFn { kind, index }) => {
                    let value = self.atom(lets, target);
                    let field = self
                        .env
                        .kinds
                        .get(kind)
                        .and_then(|k| k.record.as_ref())
                        .and_then(|fields| fields.get(index))
                        .map_or("?", |f| f.name)
                        .to_owned();
                    let f = self.temp();
                    lets.push((
                        f.clone(),
                        unknown(),
                        Bound::Rhs(Rhs::Field {
                            value,
                            field,
                            index,
                        }),
                    ));
                    let args: Vec<Atom> = args.iter().map(|a| self.atom(lets, &a.value)).collect();
                    self.push(
                        lets,
                        span,
                        Rhs::Apply {
                            callee: Atom::Var(f),
                            args,
                            span,
                        },
                    )
                }
                Some(Res::Function { id, type_args }) => {
                    let receiver = self.atom(lets, target);
                    self.call_function(lets, span, id, &type_args, Some(receiver), args)
                }
                _ => Atom::Unit,
            },
            ExprKind::Call { callee, args } => match self.res(span) {
                Some(Res::Function { id, type_args }) => {
                    self.call_function(lets, span, id, &type_args, None, args)
                }
                Some(Res::Ctor { kind, variant }) => {
                    let args = self.call_args(lets, span, args);
                    let ctor = self.ctor(kind, variant);
                    self.push(lets, span, Rhs::Construct { ctor, args })
                }
                Some(Res::Value) => {
                    let callee = self.atom(lets, callee);
                    let args: Vec<Atom> = args.iter().map(|a| self.atom(lets, &a.value)).collect();
                    self.push(lets, span, Rhs::Apply { callee, args, span })
                }
                _ => Atom::Unit,
            },
            ExprKind::CapOp { cap, op, args } => match self.res(span) {
                Some(Res::Op { capability, .. }) => {
                    let values = self.call_args(lets, span, args);
                    let labels: Vec<Label> = self
                        .ordered_arg_spans(span, args)
                        .iter()
                        .map(|arg| {
                            self.records
                                .types
                                .get(arg)
                                .map_or_else(Label::public, Ty::deep_label)
                        })
                        .collect();
                    let capability = self
                        .env
                        .capabilities
                        .get(capability)
                        .map_or("?", |c| c.name)
                        .to_owned();
                    self.push(
                        lets,
                        span,
                        Rhs::Op {
                            cap: self.name(cap).to_owned(),
                            capability,
                            op: self.name(op).to_owned(),
                            args: values,
                            labels,
                            span,
                        },
                    )
                }
                _ => Atom::Unit,
            },
            ExprKind::Result => self.result.clone().map_or(Atom::Unit, Atom::Var),
            ExprKind::It => self.it.clone().map_or(Atom::Unit, Atom::Var),
            ExprKind::Error => self.error.clone().map_or(Atom::Unit, Atom::Var),
            ExprKind::CapRef(_) => {
                self.broken = true;
                Atom::Unit
            }
        }
    }

    fn literal_atom(&mut self, lets: &mut Vec<Let>, literal: &Literal, span: Span) -> Atom {
        match literal {
            Literal::Int(digits) => Atom::Int(digits.clone()),
            Literal::Bool(value) => Atom::Bool(*value),
            Literal::Unit => Atom::Unit,
            Literal::Text(text) => self.text_atom(lets, text, span),
        }
    }

    /// A text literal; with interpolation, `concat` of its parts, each through `to_text`.
    fn text_atom(&mut self, lets: &mut Vec<Let>, text: &TextLit, span: Span) -> Atom {
        let interpolated = text
            .parts
            .iter()
            .any(|part| matches!(part, TextPart::Interpolation(_)));
        if !interpolated {
            let plain: String = text
                .parts
                .iter()
                .map(|part| match part {
                    TextPart::Text(text) => text.as_str(),
                    TextPart::Interpolation(_) => "",
                })
                .collect();
            return Atom::Text(plain);
        }
        let mut parts = Vec::new();
        for part in &text.parts {
            match part {
                TextPart::Text(chunk) => parts.push(Atom::Text(chunk.clone())),
                TextPart::Interpolation(expr) => {
                    let value = self.atom(lets, expr);
                    let t = self.temp();
                    let label = self
                        .records
                        .types
                        .get(&expr.span)
                        .map_or_else(Label::public, Ty::deep_label);
                    lets.push((
                        t.clone(),
                        with_label(text_ty(), &label),
                        Bound::Rhs(Rhs::Prim {
                            prim: Prim::ToText,
                            args: vec![value],
                        }),
                    ));
                    parts.push(Atom::Var(t));
                }
            }
        }
        self.push(
            lets,
            span,
            Rhs::Prim {
                prim: Prim::Concat,
                args: parts,
            },
        )
    }

    fn binary(
        &mut self,
        lets: &mut Vec<Let>,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        span: Span,
    ) -> Atom {
        // `and`, `or` and `implies` are `if`, so that evaluation short-circuits
        // (`spec/09 §2`).
        let short = match op {
            BinaryOp::And => Some((true, None)),
            BinaryOp::Or => Some((false, Some(true))),
            BinaryOp::Implies => Some((true, Some(true))),
            _ => None,
        };
        if let Some((then_right, constant)) = short {
            let cond = self.atom(lets, left);
            let right = self.expr(right);
            let constant = CoreExpr::Atom(Atom::Bool(constant.unwrap_or(false)));
            let (then, otherwise) = if then_right {
                (right, constant)
            } else {
                (constant, right)
            };
            let test = CoreExpr::If {
                cond,
                then: Box::new(then),
                otherwise: Box::new(otherwise),
            };
            return self.push_expr(lets, span, test);
        }
        let left = self.atom(lets, left);
        let right = self.atom(lets, right);
        let prim = match op {
            BinaryOp::Eq => Prim::Eq,
            BinaryOp::NotEq => Prim::Ne,
            BinaryOp::Lt => Prim::Lt,
            BinaryOp::Le => Prim::Le,
            BinaryOp::Gt => Prim::Gt,
            BinaryOp::Ge => Prim::Ge,
            BinaryOp::Add => Prim::Add,
            BinaryOp::Sub => Prim::Sub,
            BinaryOp::Mul => Prim::Mul,
            BinaryOp::Div => Prim::Div,
            BinaryOp::Mod => Prim::Mod,
            BinaryOp::And | BinaryOp::Or | BinaryOp::Implies => Prim::Eq,
        };
        self.push(
            lets,
            span,
            Rhs::Prim {
                prim,
                args: vec![left, right],
            },
        )
    }

    /// A call of a declared function, predicate or policy; a policy call is `declassify`.
    fn call_function(
        &mut self,
        lets: &mut Vec<Let>,
        span: Span,
        id: usize,
        type_args: &[Ty],
        receiver: Option<Atom>,
        args: &[Arg],
    ) -> Atom {
        let Some(def) = self.env.functions.get(id) else {
            self.broken = true;
            return Atom::Unit;
        };
        let values = self.ordered_values(lets, span, receiver, args);
        if def.kind == FnKind::Policy {
            let FnAst::Policy(policy) = def.ast else {
                self.broken = true;
                return Atom::Unit;
            };
            let via_name = self.env.text(def.module, policy.via.name);
            let via = match self.env.lower(def.module, via_name) {
                Some(Entity::Function(via)) => self.function_path(via),
                _ => {
                    self.broken = true;
                    return Atom::Unit;
                }
            };
            let arg = values.into_iter().next().unwrap_or(Atom::Unit);
            return self.push(
                lets,
                span,
                Rhs::Declassify {
                    policy: self.function_path(id),
                    via,
                    arg,
                    span,
                },
            );
        }
        let type_args: Vec<Type> = type_args.iter().map(|t| self.ty(t)).collect();
        let function = self.function_path(id);
        self.push(
            lets,
            span,
            Rhs::Call {
                function,
                type_args,
                args: values,
                span,
            },
        )
    }

    /// The arguments of the call at `span`, evaluated in written order and placed in
    /// parameter order (`spec/04 §3` rule 5, `spec/09 §3`).
    fn call_args(&mut self, lets: &mut Vec<Let>, span: Span, args: &[Arg]) -> Vec<Atom> {
        self.ordered_values(lets, span, None, args)
    }

    fn ordered_values(
        &mut self,
        lets: &mut Vec<Let>,
        span: Span,
        receiver: Option<Atom>,
        args: &[Arg],
    ) -> Vec<Atom> {
        let order = self.order(span);
        let mut written: Vec<Atom> = receiver.into_iter().collect();
        for arg in args {
            written.push(self.atom(lets, &arg.value));
        }
        if written.len() != order.len() {
            self.broken = true;
        }
        let count = order.iter().flatten().max().map_or(0, |max| max + 1);
        let mut slots: Vec<Option<Atom>> = vec![None; count.max(written.len())];
        for (value, target) in written.into_iter().zip(order.iter().copied()) {
            match target.and_then(|t| slots.get_mut(t)) {
                Some(slot) => *slot = Some(value),
                None => self.broken = true,
            }
        }
        slots.into_iter().flatten().collect()
    }

    /// The spans of the arguments of the call at `span`, in parameter order.
    fn ordered_arg_spans(&self, span: Span, args: &[Arg]) -> Vec<Span> {
        let order = self.order(span);
        let mut slots: Vec<Option<Span>> = vec![None; args.len()];
        for (arg, target) in args.iter().zip(order) {
            if let Some(slot) = target.and_then(|t| slots.get_mut(t)) {
                *slot = Some(arg.value.span);
            }
        }
        slots.into_iter().flatten().collect()
    }

    /// A function used as a value at `span`: `λ(x, ...). f[τ*](x, ...)`.
    fn function_value(&mut self, id: usize, type_args: &[Ty], span: Span) -> Rhs {
        let Some(def) = self.env.functions.get(id) else {
            self.broken = true;
            return Rhs::Atom(Atom::Unit);
        };
        let params: Vec<(Name, Type)> = def
            .params
            .iter()
            .map(|p| {
                let ty = self.ty(&p.ty.base.substitute(type_args));
                let t = self.temp();
                (t, ty)
            })
            .collect();
        let args: Vec<Atom> = params.iter().map(|(n, _)| Atom::Var(n.clone())).collect();
        let t = self.temp();
        let result = self.ty(&def.result.base.substitute(type_args));
        let body = CoreExpr::Let {
            name: t.clone(),
            ty: result,
            value: Box::new(Bound::Rhs(Rhs::Call {
                function: self.function_path(id),
                type_args: type_args.iter().map(|a| self.ty(a)).collect(),
                args,
                span,
            })),
            body: Box::new(CoreExpr::Atom(Atom::Var(t))),
        };
        Rhs::Lambda {
            params,
            body: Box::new(body),
        }
    }
}

// ----- helpers -----

fn close(lets: Vec<Let>, tail: CoreExpr) -> CoreExpr {
    lets.into_iter()
        .rev()
        .fold(tail, |body, (name, ty, value)| CoreExpr::Let {
            name,
            ty,
            value: Box::new(value),
            body: Box::new(body),
        })
}

/// Binds `name` to `expr`, flattening a `let` chain that ends in an atom into `lets`.
fn bind_expr(lets: &mut Vec<Let>, name: Name, ty: Type, expr: CoreExpr) {
    let mut current = expr;
    loop {
        match current {
            CoreExpr::Let {
                name: inner,
                ty: inner_ty,
                value,
                body,
            } => {
                lets.push((inner, inner_ty, *value));
                current = *body;
            }
            CoreExpr::Atom(Atom::Var(var))
                if lets.last().is_some_and(|(last, _, _)| *last == var) =>
            {
                if let Some((_, _, value)) = lets.pop() {
                    lets.push((name, ty, value));
                }
                return;
            }
            CoreExpr::Atom(atom) => {
                lets.push((name, ty, Bound::Rhs(Rhs::Atom(atom))));
                return;
            }
            other => {
                lets.push((name, ty, Bound::Expr(other)));
                return;
            }
        }
    }
}

fn with_label(ty: Type, label: &Label) -> Type {
    match ty {
        Type::Base {
            base,
            refinements,
            label: own,
        } => Type::Base {
            base,
            refinements,
            label: own.join(label),
        },
        Type::Fn {
            params,
            result,
            uses,
            fails,
            label: own,
        } => Type::Fn {
            params,
            result,
            uses,
            fails,
            label: own.join(label),
        },
    }
}

fn plain(base: Base) -> Type {
    Type::Base {
        base,
        refinements: Vec::new(),
        label: Label::public(),
    }
}

fn unknown() -> Type {
    plain(Base::Unknown)
}

fn unit() -> Type {
    plain(Base::Unit)
}

fn bool_ty() -> Type {
    plain(Base::Bool)
}

fn text_ty() -> Type {
    plain(Base::Text)
}

fn signed(value: &SignedInt) -> String {
    if value.negative {
        format!("-{}", value.digits)
    } else {
        value.digits.clone()
    }
}
