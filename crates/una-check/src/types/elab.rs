//! Elaboration to Core IR (`spec/09 §3`): the desugaring of the declarations that checked
//! without diagnostics, from what pass 2 resolved, with the formulas and
//! spans that verification needs.

use std::collections::BTreeMap;

use una_core::{
    Arm as CoreArm, Atom, Base, BinOp, Bound, CapOp as CoreCapOp, Clause as CoreClause, Ctor,
    DataVariant, Decl, Example, Expectation, Expr as CoreExpr, Fun, HandlerOp as CoreHandlerOp,
    Label, Module as CoreModule, Name, Path, Prim, Program, Refinement as CoreRefinement, Rhs,
    Signature, Source, Term, Type, Use,
};
use una_syntax::ast::{
    Arg, BinaryOp, Block, BlockExpr, Body, CapBindingValue, Clause, ClauseKind, DeclKind,
    ElseBranch, ExampleDecl, ExpectationKind, Expr, ExprKind, FunctionDecl, Handled, HandlingKind,
    IfBlock, IsTarget, LetValue, Literal, OpClauseKind, PatternKind, Range, SignedInt, Stmt,
    TextLit, TextPart, UnaryOp, VariantExpr, WhenExpr,
};
use una_syntax::{SourceDb, Span};

use super::Findings;
use super::env::{Entity, Env, FnAst, FnKind, Need};
use super::records::{Records, Res};
use super::ty::{Declared, FnUse, Refinement, Ty};

/// One binding of a `let` chain being built.
struct Let {
    name: Name,
    ty: Type,
    value: Bound,
    /// The source of the value.
    span: Span,
    /// Whether the source writes the binder's type.
    written: bool,
}

impl Let {
    /// A binding whose type the source does not write: a temporary, or a `let` without an
    /// annotation.
    fn inferred(name: Name, ty: Type, value: Bound, span: Span) -> Let {
        Let {
            name,
            ty,
            value,
            span,
            written: false,
        }
    }
}

/// Elaborates the package's modules, in path order, and the contracts of the standard
/// library; a declaration with a diagnostic, or with a resolution missing, is left out.
/// Returns the program, and the components of checker types that a Core IR binder type
/// lost, with the span of their declaration ([`lost`]).
pub(crate) fn program(
    env: &Env<'_>,
    findings: &Findings,
    sources: &SourceDb,
    standard: &SourceDb,
) -> (Program, Vec<(Span, String)>) {
    let dbs = Dbs { sources, standard };
    let mut lost = Vec::new();
    let mut modules: Vec<CoreModule> = (0..env.prelude)
        .filter(|module| env.reports(*module))
        .map(|module| elaborate_module(env, findings, dbs, module, &mut lost))
        .collect();
    modules.sort_by(|a, b| a.path.cmp(&b.path));
    let library = (env.prelude..env.modules.len())
        .flat_map(|module| library(env, findings, dbs, module))
        .collect();
    let host = host(env, findings, dbs);
    (
        Program {
            modules,
            library,
            host,
        },
        lost,
    )
}

/// The host capabilities, which the prelude declares for `std.host` (`spec/14 §3`).
fn host(env: &Env<'_>, findings: &Findings, dbs: Dbs<'_>) -> Vec<Decl> {
    let Some(m) = env.modules.get(env.prelude) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (decl, entity) in m.tree.decls.iter().zip(m.decls.clone()) {
        if let (DeclKind::Capability(_), Some(entity @ Entity::Capability(_))) =
            (&decl.kind, entity)
        {
            let mut fx = Fx::new(
                env,
                records_of(env, findings, env.prelude),
                dbs,
                env.prelude,
            );
            if let Some(core) = fx.decl(&decl.kind, entity) {
                out.push(core);
            }
        }
    }
    out
}

/// The records of module `module`: those of the package, or those of the standard modules,
/// whose spans are those of another source database.
fn records_of<'f>(env: &Env<'_>, findings: &'f Findings, module: usize) -> &'f Records {
    if module >= env.prelude {
        &findings.standard_records
    } else {
        &findings.records
    }
}

/// The sources of the package, and those of the standard modules, which have a database of
/// their own.
#[derive(Clone, Copy)]
struct Dbs<'e> {
    sources: &'e SourceDb,
    standard: &'e SourceDb,
}

fn elaborate_module(
    env: &Env<'_>,
    findings: &Findings,
    dbs: Dbs<'_>,
    module: usize,
    lost: &mut Vec<(Span, String)>,
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
        let mut fx = Fx::new(env, records_of(env, findings, module), dbs, module);
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
        lost.extend(fx.lost.take().into_iter().map(|what| (decl.span, what)));
    }
    out
}

/// The signatures of the functions of a standard module.
fn library(env: &Env<'_>, findings: &Findings, dbs: Dbs<'_>, module: usize) -> Vec<Signature> {
    let Some(m) = env.modules.get(module) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (decl, entity) in m.tree.decls.iter().zip(m.decls.clone()) {
        if let (DeclKind::Function(function), Some(Entity::Function(id))) = (&decl.kind, entity) {
            let mut fx = Fx::new(env, records_of(env, findings, module), dbs, module);
            if let Some(signature) = fx.signature(function, id) {
                out.push(signature);
            }
        }
    }
    out
}

/// The elaboration of one declaration.
struct Fx<'e, 'a> {
    env: &'e Env<'a>,
    records: &'e Records,
    dbs: Dbs<'e>,
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
    /// What the Core IR types of this declaration lost of the checker types they come from
    /// (ADR 0038 D1, the consistency check of `spec/10 §2`).
    lost: std::cell::RefCell<Vec<String>>,
}

impl<'e, 'a> Fx<'e, 'a> {
    fn new(env: &'e Env<'a>, records: &'e Records, dbs: Dbs<'e>, module: usize) -> Self {
        Fx {
            env,
            records,
            dbs,
            module,
            type_params: Vec::new(),
            scope: Vec::new(),
            used: BTreeMap::new(),
            temps: 0,
            error: None,
            it: None,
            result: None,
            broken: false,
            lost: std::cell::RefCell::new(Vec::new()),
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

    /// The Core IR type of a checker type. In every build, the result is compared with `ty`,
    /// and a component that it lost is recorded, for U8001 (ADR 0038 D1, ADR 0045 rule 3).
    fn ty(&self, ty: &Ty) -> Type {
        let core = self.convert(ty);
        if let Some(what) = lost(ty, &core) {
            self.lost.borrow_mut().push(what);
        }
        core
    }

    fn convert(&self, ty: &Ty) -> Type {
        match ty {
            Ty::Labeled(label, inner) => with_label(self.convert(inner), label),
            // ADR 0037 rule 1: the parameters and the result keep their refinements.
            Ty::Fn(function) => Type::Fn {
                params: function
                    .params
                    .iter()
                    .enumerate()
                    .map(|(index, p)| {
                        self.refined(self.convert(p), function.refinements.param(index))
                    })
                    .collect(),
                result: Box::new(
                    self.refined(self.convert(&function.result), &function.refinements.result),
                ),
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
                module: "prelude".to_owned(),
                name: "List".to_owned(),
                args: vec![self.convert(element)],
            },
            Ty::Kind(id, args) => Base::Named {
                module: self
                    .env
                    .kinds
                    .get(*id)
                    .and_then(|kind| self.env.modules.get(kind.module))
                    .map(|module| module.path.clone())
                    .unwrap_or_default(),
                name: self.env.kind_name(*id).to_owned(),
                args: args.iter().map(|a| self.convert(a)).collect(),
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

    /// A declared type, with the refinements of the kinds and ranges it names. Core IR has no
    /// room for the refinement of a function-typed value (`spec/09 §2`); such a `where` clause
    /// is U2002 (ADR 0045), and one that still reaches here is recorded as lost in every
    /// build, for U8001 (ADR 0037 rule 11), never dropped silently.
    fn declared(&mut self, declared: &Declared) -> Type {
        let ty = self.ty(&declared.base);
        self.refined_kept(ty, &declared.refinements)
    }

    /// [`Fx::refined`], recording in every build a refinement that Core IR cannot hold.
    fn refined_kept(&self, ty: Type, refinements: &[Refinement]) -> Type {
        let ty = self.refined(ty, refinements);
        let kept = match &ty {
            Type::Base { refinements, .. } => refinements.len(),
            Type::Fn { .. } => 0,
        };
        if kept < refinements.len() {
            self.lost
                .borrow_mut()
                .push("the refinement of a declared type".to_owned());
        }
        ty
    }

    /// `ty` with `refinements` elaborated into its refinements; a function type has no room
    /// for them, and [`lost`] or [`Fx::declared`] report what is left out.
    fn refined(&self, mut ty: Type, refinements: &[Refinement]) -> Type {
        if refinements.is_empty() {
            return ty;
        }
        let mut fx = Fx::new(self.env, self.records, self.dbs, self.module);
        fx.type_params.clone_from(&self.type_params);
        let core: Vec<CoreRefinement> = refinements
            .iter()
            .filter_map(|refinement| fx.refinement(refinement))
            .collect();
        if let Type::Base {
            refinements: slot, ..
        } = &mut ty
        {
            *slot = core;
        }
        ty
    }

    /// The Core IR type of the lambda parameter whose name is at `span`, with the refinements
    /// the checker recorded for it (ADR 0037 rule 1).
    fn lambda_param(&self, span: Span) -> Type {
        let ty = self.type_at(span);
        match self.records.lambda_params.get(&span) {
            Some(refinements) => self.refined_kept(ty, refinements),
            None => ty,
        }
    }

    /// A refinement with its predicate over `it`: a range, or the `where` clause of an alias
    /// kind, elaborated in the alias's module.
    fn refinement(&mut self, refinement: &Refinement) -> Option<CoreRefinement> {
        match refinement {
            Refinement::Range(text, span) => {
                let (low, high, exclusive) = parse_range(text)?;
                Some(CoreRefinement {
                    name: text.clone(),
                    clause: CoreClause {
                        term: Term::In {
                            value: Box::new(Term::It),
                            low,
                            high,
                            exclusive,
                        },
                        text: text.clone(),
                        span: self.env.in_package(self.module).then_some(*span),
                    },
                })
            }
            Refinement::Alias(id, _) => {
                let alias = self.env.aliases.get(*id)?;
                let expr = alias.refinement?;
                let scope = std::mem::take(&mut self.scope);
                let module = std::mem::replace(&mut self.module, alias.module);
                let clause = self.clause(expr);
                self.module = module;
                self.scope = scope;
                Some(CoreRefinement {
                    name: alias.name.to_owned(),
                    clause,
                })
            }
        }
    }

    fn kind_name(&self, ty: &Ty) -> String {
        match ty {
            Ty::Kind(id, _) => self.env.kind_name(*id).to_owned(),
            Ty::Labeled(_, inner) => self.kind_name(inner),
            _ => "?".to_owned(),
        }
    }

    /// The qualified identity of capability `id` (`spec/10 §11` rule 8): the capabilities of
    /// the prelude are the host capabilities, declared in `std.host` (`spec/14 §3`).
    fn capability_path(&self, id: Option<usize>) -> Path {
        let Some(def) = id.and_then(|id| self.env.capabilities.get(id)) else {
            return Path {
                module: String::new(),
                name: "?".to_owned(),
            };
        };
        let module = if def.module == self.env.prelude {
            una_core::HOST_MODULE.to_owned()
        } else {
            self.env
                .modules
                .get(def.module)
                .map(|m| m.path.clone())
                .unwrap_or_default()
        };
        Path {
            module,
            name: def.name.to_owned(),
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

    /// The kind and the name of field `index` of record kind `kind`.
    fn field(&self, kind: usize, index: usize) -> (Path, String) {
        match self.env.kinds.get(kind) {
            Some(def) => (
                self.path(def.module, def.name),
                def.record
                    .as_ref()
                    .and_then(|fields| fields.get(index))
                    .map_or("?", |f| f.name)
                    .to_owned(),
            ),
            None => (self.path(self.module, "?"), "?".to_owned()),
        }
    }

    /// The Core IR type of field `index` of record kind `kind`, a function type, read from the
    /// record at `record`: the declared type at the record's type arguments, with its label.
    fn field_fn_type(&mut self, kind: usize, index: usize, record: Span) -> Type {
        let record = self
            .records
            .types
            .get(&record)
            .cloned()
            .unwrap_or(Ty::Error);
        let args = match record.unlabeled() {
            Ty::Kind(_, args) => args.clone(),
            _ => Vec::new(),
        };
        let declared = self
            .env
            .kinds
            .get(kind)
            .and_then(|def| def.record.as_ref())
            .and_then(|fields| fields.get(index))
            .map(|field| Declared {
                base: field
                    .ty
                    .base
                    .substitute(&args)
                    .with_label(&record.top_label()),
                refinements: field.ty.refinements.clone(),
            });
        match declared {
            Some(declared) => self.declared(&declared),
            None => unknown(),
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

    // ----- the refinement language -----

    /// A formula of the refinement language with its text and span; a clause of a standard
    /// module has no span, since its source is never reported.
    fn clause(&mut self, expr: &Expr) -> CoreClause {
        CoreClause {
            term: self.term(expr),
            text: self.text(expr.span),
            span: self.env.in_package(self.module).then_some(expr.span),
        }
    }

    /// The `requires` and `ensures` clauses of a function header.
    fn contracts(&mut self, clauses: &[Clause]) -> (Vec<CoreClause>, Vec<CoreClause>) {
        let mut requires = Vec::new();
        let mut ensures = Vec::new();
        for clause in clauses {
            match &clause.kind {
                ClauseKind::Requires(expr) => requires.push(self.clause(expr)),
                ClauseKind::Ensures(expr) => ensures.push(self.clause(expr)),
                ClauseKind::Uses(_) | ClauseKind::Fails(_) | ClauseKind::DerivedFrom(_) => {}
            }
        }
        (requires, ensures)
    }

    /// An expression of the refinement language as a term (`spec/03 §3`).
    /// The checker has already rejected every other form (U2014).
    fn term(&mut self, expr: &Expr) -> Term {
        match &expr.kind {
            ExprKind::Paren(inner) => self.term(inner),
            ExprKind::Literal(literal) => self.literal_term(literal),
            ExprKind::Name(ident) => {
                let name = self.name(ident);
                match self.local(name) {
                    Some(binder) => Term::Var(binder),
                    None => self.outside(),
                }
            }
            ExprKind::UpperName(ident) => match self.res(ident.span) {
                Some(Res::Ctor { kind, variant }) => Term::Ctor(self.ctor(kind, variant)),
                _ => self.outside(),
            },
            ExprKind::Result => Term::Result,
            ExprKind::It => Term::It,
            ExprKind::Binary { op, left, right } => Term::Binary {
                op: bin_op(*op),
                left: Box::new(self.term(left)),
                right: Box::new(self.term(right)),
            },
            ExprKind::Unary { op, operand } => {
                let operand = Box::new(self.term(operand));
                match op {
                    UnaryOp::Neg => Term::Neg(operand),
                    UnaryOp::Not => Term::Not(operand),
                }
            }
            ExprKind::Is { value, target } => {
                let value = Box::new(self.term(value));
                match target {
                    IsTarget::Variant(ident) => match self.res(ident.span) {
                        Some(Res::Ctor { kind, variant }) => Term::Is {
                            value,
                            ctor: self.ctor(kind, variant),
                        },
                        _ => self.outside(),
                    },
                    IsTarget::Literal(literal) => Term::Binary {
                        op: BinOp::Eq,
                        left: value,
                        right: Box::new(self.literal_term(literal)),
                    },
                }
            }
            ExprKind::In { value, range } => Term::In {
                value: Box::new(self.term(value)),
                low: signed(&range.low),
                high: signed(&range.high),
                exclusive: range.exclusive,
            },
            ExprKind::Field { target, .. } => match self.res(expr.span) {
                Some(Res::Len) => Term::Len(Box::new(self.term(target))),
                Some(Res::Field { kind, index }) => {
                    let (kind, field) = self.field(kind, index);
                    Term::Field {
                        value: Box::new(self.term(target)),
                        kind,
                        field,
                        index,
                    }
                }
                Some(Res::Function { id, .. }) => {
                    let receiver = self.term(target);
                    Term::Call {
                        predicate: self.function_path(id),
                        args: vec![receiver],
                    }
                }
                _ => self.outside(),
            },
            ExprKind::MethodCall { target, args, .. } => match self.res(expr.span) {
                Some(Res::Function { id, .. }) => {
                    let receiver = self.term(target);
                    let args = self.ordered_terms(expr.span, Some(receiver), args);
                    Term::Call {
                        predicate: self.function_path(id),
                        args,
                    }
                }
                _ => self.outside(),
            },
            ExprKind::Call { args, .. } => match self.res(expr.span) {
                Some(Res::Function { id, .. }) => {
                    let args = self.ordered_terms(expr.span, None, args);
                    Term::Call {
                        predicate: self.function_path(id),
                        args,
                    }
                }
                _ => self.outside(),
            },
            _ => self.outside(),
        }
    }

    fn literal_term(&mut self, literal: &Literal) -> Term {
        match literal {
            Literal::Int(digits) => Term::Int(digits.clone()),
            Literal::Bool(value) => Term::Bool(*value),
            Literal::Unit => Term::Unit,
            Literal::Text(text) => match plain_text(text) {
                Some(text) => Term::Text(text),
                None => self.outside(),
            },
        }
    }

    /// A form outside the refinement language, which the checker reports (U2014): the
    /// declaration is not elaborated.
    fn outside(&mut self) -> Term {
        self.broken = true;
        Term::Bool(false)
    }

    /// The arguments of the call at `span` as terms, in parameter order.
    fn ordered_terms(&mut self, span: Span, receiver: Option<Term>, args: &[Arg]) -> Vec<Term> {
        let order = self.order(span);
        let mut written: Vec<Term> = receiver.into_iter().collect();
        for arg in args {
            written.push(self.term(&arg.value));
        }
        if written.len() != order.len() {
            self.broken = true;
        }
        let count = order.iter().flatten().max().map_or(0, |max| max + 1);
        let mut slots: Vec<Option<Term>> = vec![None; count.max(written.len())];
        for (value, target) in written.into_iter().zip(order.iter().copied()) {
            match target.and_then(|t| slots.get_mut(t)) {
                Some(slot) => *slot = Some(value),
                None => self.broken = true,
            }
        }
        slots.into_iter().flatten().collect()
    }

    // ----- declarations -----

    /// The contract of a function of a standard module.
    fn signature(&mut self, function: &'a FunctionDecl, id: usize) -> Option<Signature> {
        let def = self.env.functions.get(id)?;
        self.type_params = def.type_params.clone();
        let params = self.params(&def.params);
        let result = self.declared(&def.result);
        let (requires, ensures) = self.contracts(&function.clauses);
        (!self.broken).then(|| Signature {
            path: self.path(def.module, def.name),
            type_params: def.type_params.iter().map(|p| (*p).to_owned()).collect(),
            params,
            result,
            requires,
            ensures,
        })
    }

    fn decl(&mut self, kind: &'a DeclKind, entity: Entity) -> Option<Decl> {
        let decl = match (kind, entity) {
            (DeclKind::Kind(_), Entity::Alias(id)) => {
                let alias = self.env.aliases.get(id)?;
                self.type_params = alias.params.clone();
                let target = self.declared(&alias.base);
                let target = match alias.refinement {
                    Some(_) => {
                        let own = self.refinement(&Refinement::Alias(id, Vec::new()));
                        match target {
                            Type::Base {
                                base,
                                mut refinements,
                                label,
                            } => {
                                refinements.extend(own);
                                Type::Base {
                                    base,
                                    refinements,
                                    label,
                                }
                            }
                            other => other,
                        }
                    }
                    None => target,
                };
                Decl::Alias {
                    name: alias.name.to_owned(),
                    params: alias.params.iter().map(|p| (*p).to_owned()).collect(),
                    target,
                }
            }
            (DeclKind::Kind(_), Entity::Kind(id)) => {
                let def = self.env.kinds.get(id)?;
                self.type_params = def.params.clone();
                let (variants, record) = match &def.record {
                    Some(fields) => {
                        let fields = fields
                            .iter()
                            .map(|f| (f.name.to_owned(), self.declared(&f.ty)))
                            .collect();
                        (
                            vec![DataVariant {
                                name: def.name.to_owned(),
                                fields,
                            }],
                            true,
                        )
                    }
                    None => {
                        let mut variants = Vec::new();
                        for v in &def.variants {
                            let fields = v
                                .fields
                                .iter()
                                .map(|f| (f.name.to_owned(), self.declared(&f.ty)))
                                .collect();
                            variants.push(DataVariant {
                                name: v.name.to_owned(),
                                fields,
                            });
                        }
                        (variants, false)
                    }
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
                let result = self.declared(&def.result);
                let (requires, ensures) = self.contracts(&function.clauses);
                let body = self.body(&function.body);
                Decl::Fun(Fun {
                    name: def.name.to_owned(),
                    type_params: def.type_params.iter().map(|p| (*p).to_owned()).collect(),
                    params,
                    result,
                    uses: self.needs(&def.uses),
                    fails: def.fails.as_ref().map(|f| self.kind_name(f)),
                    requires,
                    ensures,
                    body,
                    span: function.name.span,
                    public: function.public,
                })
            }
            (DeclKind::Predicate(predicate), Entity::Function(id)) => {
                let def = self.env.functions.get(id)?;
                let params = self.params(&def.params);
                let reflect = matches!(def.kind, FnKind::Predicate { reflect: true });
                let formula = match (&predicate.body, reflect) {
                    (Body::Line(expr), true) => Some(self.term(expr)),
                    (Body::Block(_), true) => Some(self.outside()),
                    (_, false) => None,
                };
                let body = self.body(&predicate.body);
                Decl::Pred {
                    name: def.name.to_owned(),
                    reflect,
                    params,
                    body,
                    formula,
                    span: predicate.name.span,
                }
            }
            (DeclKind::Capability(capability), Entity::Capability(id)) => {
                let def = self.env.capabilities.get(id)?;
                let mut ops = Vec::new();
                for (op, ast) in def.ops.iter().zip(&capability.ops) {
                    let mark = self.scope.len();
                    let params: Vec<(Name, Type)> = op
                        .params
                        .iter()
                        .map(|p| {
                            let ty = self.declared(&p.ty);
                            (self.bind(p.name), ty)
                        })
                        .collect();
                    let result = self.declared(&op.result);
                    let mut requires = Vec::new();
                    let mut ensures = Vec::new();
                    for clause in &ast.clauses {
                        match &clause.kind {
                            OpClauseKind::Requires(expr) => requires.push(self.clause(expr)),
                            OpClauseKind::Ensures(expr) => ensures.push(self.clause(expr)),
                            OpClauseKind::Fails(_) | OpClauseKind::Accepts(_) => {}
                        }
                    }
                    self.scope.truncate(mark);
                    ops.push(CoreCapOp {
                        name: op.name.to_owned(),
                        params,
                        result,
                        fails: op.fails.as_ref().map(|f| self.kind_name(f)),
                        requires,
                        ensures,
                        accepts: op.accepts.clone(),
                    });
                }
                Decl::Cap {
                    name: def.name.to_owned(),
                    ops,
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
                    capability_path: self.capability_path(def.capability),
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
                let from = match def.params.first() {
                    Some(param) => self.declared(&param.ty),
                    None => unknown(),
                };
                Decl::Policy {
                    name: def.name.to_owned(),
                    from,
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
            let (args, arg_spans) =
                self.call_args(&mut lets, given.handler.span, &given.handler.args);
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
                    arg_spans,
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
            let line = self.dbs.sources.location(expectation.span).start.line;
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

    /// The source text at `span`, in the database of the current module.
    fn text(&self, span: Span) -> String {
        let db = if self.env.in_package(self.module) {
            self.dbs.sources
        } else {
            self.dbs.standard
        };
        db.file(span.file)
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
        let mut tail = CoreExpr::Atom {
            atom: Atom::Unit,
            span: block.span,
        };
        for (index, stmt) in block.stmts.iter().enumerate() {
            match stmt {
                Stmt::Let(stmt) => {
                    let (value, span) = match &stmt.value {
                        LetValue::Handled(handled) => (self.handled(handled), handled.expr.span),
                        LetValue::BlockExpr(expr) => (self.block_expr(expr), block_expr_span(expr)),
                    };
                    let written = self.records.annotations.get(&stmt.name.span).cloned();
                    let ty = match &written {
                        Some(declared) => self.declared(declared),
                        None => self.type_at(stmt.name.span),
                    };
                    let name = self.name(&stmt.name);
                    let binder = self.bind(name);
                    bind_expr(&mut lets, binder, ty, value, span, written.is_some());
                }
                Stmt::Handled(handled) if index == last => tail = self.handled(handled),
                Stmt::BlockExpr(expr) if index == last => tail = self.block_expr(expr),
                Stmt::Handled(handled) => {
                    let value = self.handled(handled);
                    bind_expr(
                        &mut lets,
                        "_".to_owned(),
                        unit(),
                        value,
                        handled.expr.span,
                        false,
                    );
                }
                Stmt::BlockExpr(expr) => {
                    let value = self.block_expr(expr);
                    bind_expr(
                        &mut lets,
                        "_".to_owned(),
                        unit(),
                        value,
                        block_expr_span(expr),
                        false,
                    );
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
        let span = handled.expr.span;
        for binding in &handled.with {
            let cap = self.name(&binding.cap).to_owned();
            let source = match &binding.value {
                CapBindingValue::Cap(other) => Source::Rebind(self.name(other).to_owned()),
                CapBindingValue::Handler(call) => {
                    let handler = match self.res(call.span) {
                        Some(Res::Handler { id }) => id,
                        _ => {
                            return CoreExpr::Atom {
                                atom: Atom::Unit,
                                span,
                            };
                        }
                    };
                    let (args, arg_spans) = self.call_args(&mut lets, call.span, &call.args);
                    let module = self
                        .env
                        .handlers
                        .get(handler)
                        .map_or(self.module, |h| h.module);
                    Source::Handler {
                        handler: self.path(module, self.name(&call.name)),
                        args,
                        arg_spans,
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
                    self.unwrap_option(current, value_ty.clone(), otherwise, span)
                }
                HandlingKind::NoneUse(value) => {
                    let otherwise = self.expr(value);
                    self.unwrap_option(current, value_ty.clone(), otherwise, span)
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
    fn unwrap_option(
        &mut self,
        option: CoreExpr,
        ty: Type,
        otherwise: CoreExpr,
        span: Span,
    ) -> CoreExpr {
        let Some(kind) = self.env.prelude_kind("Option") else {
            self.broken = true;
            return option;
        };
        let t = self.temp();
        let v = self.temp();
        let mut lets = Vec::new();
        bind_expr(&mut lets, t.clone(), ty, option, span, false);
        let some = self.ctor(kind, Some(0));
        close(
            lets,
            CoreExpr::Case {
                scrutinee: Atom::Var(t),
                arms: vec![CoreArm {
                    ctor: some,
                    binds: vec![Some(v.clone())],
                    body: CoreExpr::Atom {
                        atom: Atom::Var(v),
                        span,
                    },
                }],
                otherwise: Some(Box::new(otherwise)),
            },
        )
    }

    fn fail(&mut self, variant: &VariantExpr) -> CoreExpr {
        let mut lets = Vec::new();
        let (kind, index) = match self.res(variant.span) {
            Some(Res::Ctor { kind, variant }) => (kind, variant),
            _ => {
                return CoreExpr::Atom {
                    atom: Atom::Unit,
                    span: variant.span,
                };
            }
        };
        let (args, arg_spans) = self.call_args(&mut lets, variant.span, &variant.args);
        let ctor = self.ctor(kind, index);
        close(
            lets,
            CoreExpr::Fail {
                ctor,
                args,
                arg_spans,
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
    /// final branch. Without `else`, the chain ends in `unreachable`, which the `exhaustive`
    /// obligation proves is never taken (`spec/12 §3`).
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
        let scrutinee_text = self.text(when.scrutinee.span);
        let conditions: Vec<String> = when
            .arms
            .iter()
            .filter_map(|arm| match &arm.pattern.kind {
                // The pattern's span covers `is`; the goal prints the literal alone (R-032).
                PatternKind::Literal(_) => {
                    let text = self.text(arm.pattern.span);
                    let value = text
                        .strip_prefix("is")
                        .map_or(text.as_str(), str::trim_start);
                    Some(format!("{scrutinee_text} == {value}"))
                }
                PatternKind::Range(range) => {
                    Some(format!("{scrutinee_text} in {}", self.text(range.span)))
                }
                PatternKind::Else | PatternKind::Variant { .. } => None,
            })
            .collect();
        let mut chain: Option<CoreExpr> = None;
        let has_else = when
            .arms
            .iter()
            .any(|arm| matches!(arm.pattern.kind, PatternKind::Else));
        if !has_else {
            chain = Some(CoreExpr::Unreachable {
                span: when.keyword,
                goal: conditions.join(" or "),
            });
        }
        for arm in when.arms.iter().rev() {
            let body = self.body(&arm.body);
            let test = match &arm.pattern.kind {
                PatternKind::Literal(literal) => {
                    let mut test_lets = Vec::new();
                    let value = self.literal_atom(&mut test_lets, literal, arm.pattern.span);
                    let t = self.temp();
                    test_lets.push(Let::inferred(
                        t.clone(),
                        bool_ty(),
                        Bound::Rhs(Rhs::Prim {
                            prim: Prim::Eq,
                            args: vec![scrutinee.clone(), value],
                            arg_spans: vec![when.scrutinee.span, arm.pattern.span],
                            span: arm.pattern.span,
                        }),
                        arm.pattern.span,
                    ));
                    Some((test_lets, Atom::Var(t)))
                }
                PatternKind::Range(range) => {
                    let mut test_lets = Vec::new();
                    let t = self.range_test(&mut test_lets, &scrutinee, when.scrutinee.span, range);
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
        close(
            lets,
            chain.unwrap_or(CoreExpr::Atom {
                atom: Atom::Unit,
                span: when.span,
            }),
        )
    }

    /// `lo <= x and x <= hi`, or `x < hi` for `..<`, as nested `if`s.
    fn range_test(
        &mut self,
        lets: &mut Vec<Let>,
        value: &Atom,
        value_span: Span,
        range: &Range,
    ) -> Atom {
        let low = Atom::Int(signed(&range.low));
        let high = Atom::Int(signed(&range.high));
        let span = range.span;
        let above = self.temp();
        lets.push(Let::inferred(
            above.clone(),
            bool_ty(),
            Bound::Rhs(Rhs::Prim {
                prim: Prim::Le,
                args: vec![low, value.clone()],
                arg_spans: vec![span, value_span],
                span,
            }),
            span,
        ));
        let below = self.temp();
        let prim = if range.exclusive { Prim::Lt } else { Prim::Le };
        let below_expr = CoreExpr::Let {
            name: below.clone(),
            ty: bool_ty(),
            value: Box::new(Bound::Rhs(Rhs::Prim {
                prim,
                args: vec![value.clone(), high],
                arg_spans: vec![value_span, span],
                span,
            })),
            body: Box::new(CoreExpr::Atom {
                atom: Atom::Var(below),
                span,
            }),
            span,
            written: false,
        };
        let t = self.temp();
        lets.push(Let::inferred(
            t.clone(),
            bool_ty(),
            Bound::Expr(CoreExpr::If {
                cond: Atom::Var(above),
                then: Box::new(below_expr),
                otherwise: Box::new(CoreExpr::Atom {
                    atom: Atom::Bool(false),
                    span,
                }),
            }),
            span,
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
            ExprKind::Attempt(operand) => CoreExpr::Attempt {
                body: Box::new(self.expr(operand)),
                span: expr.span,
            },
            _ => {
                let mut lets = Vec::new();
                let atom = self.atom(&mut lets, expr);
                close(
                    lets,
                    CoreExpr::Atom {
                        atom,
                        span: expr.span,
                    },
                )
            }
        }
    }

    /// Binds `rhs` to a fresh temporary of the type recorded at `span`.
    fn push(&mut self, lets: &mut Vec<Let>, span: Span, rhs: Rhs) -> Atom {
        let t = self.temp();
        let ty = self.type_at(span);
        lets.push(Let::inferred(t.clone(), ty, Bound::Rhs(rhs), span));
        Atom::Var(t)
    }

    /// Binds an expression to a fresh temporary of the type recorded at `span`.
    fn push_expr(&mut self, lets: &mut Vec<Let>, span: Span, expr: CoreExpr) -> Atom {
        let t = self.temp();
        let ty = self.type_at(span);
        bind_expr(lets, t.clone(), ty, expr, span, false);
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
                            arg_spans: Vec::new(),
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
                let operand_span = operand.span;
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
                        arg_spans: vec![operand_span],
                        span,
                    },
                )
            }
            ExprKind::Is { value, target } => {
                let value_span = value.span;
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
                                    body: CoreExpr::Atom {
                                        atom: Atom::Bool(true),
                                        span,
                                    },
                                }],
                                otherwise: Some(Box::new(CoreExpr::Atom {
                                    atom: Atom::Bool(false),
                                    span,
                                })),
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
                                arg_spans: vec![value_span, span],
                                span,
                            },
                        )
                    }
                }
            }
            ExprKind::In { value, range } => {
                let value_span = value.span;
                let value = self.atom(lets, value);
                self.range_test(lets, &value, value_span, range)
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
                        let ty = self.lambda_param(p.name.span);
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
                        function: None,
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
                let predicate_expr = self.expr(predicate);
                self.it = saved;
                self.scope.pop();
                let formula = self.term(predicate);
                let (kind, index) = match self.res(failure.span) {
                    Some(Res::Ctor { kind, variant }) => (kind, variant),
                    _ => return Atom::Unit,
                };
                let (args, arg_spans) = self.call_args(lets, failure.span, &failure.args);
                let ctor = self.ctor(kind, index);
                self.push(
                    lets,
                    span,
                    Rhs::Checked {
                        value,
                        it,
                        predicate: Box::new(predicate_expr),
                        formula,
                        ctor,
                        args,
                        arg_spans,
                        span,
                    },
                )
            }
            ExprKind::Field { target, .. } => match self.res(span) {
                Some(Res::Len) => {
                    let target_span = target.span;
                    let value = self.atom(lets, target);
                    self.push(
                        lets,
                        span,
                        Rhs::Prim {
                            prim: Prim::Len,
                            args: vec![value],
                            arg_spans: vec![target_span],
                            span,
                        },
                    )
                }
                Some(Res::Field { kind, index }) => {
                    let value = self.atom(lets, target);
                    let (_, field) = self.field(kind, index);
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
                    let receiver = (receiver, target.span);
                    self.call_function(lets, span, id, &type_args, Some(receiver), &[])
                }
                _ => Atom::Unit,
            },
            ExprKind::MethodCall { target, args, .. } => match self.res(span) {
                Some(Res::FieldFn { kind, index }) => {
                    let value = self.atom(lets, target);
                    let (_, field) = self.field(kind, index);
                    // ADR 0037 rule 7: the field's declared function type, refinements
                    // included, at the record's type arguments and label.
                    let field_ty = self.field_fn_type(kind, index, target.span);
                    let f = self.temp();
                    lets.push(Let::inferred(
                        f.clone(),
                        field_ty,
                        Bound::Rhs(Rhs::Field {
                            value,
                            field,
                            index,
                        }),
                        target.span,
                    ));
                    let arg_spans: Vec<Span> = args.iter().map(|a| a.value.span).collect();
                    let args: Vec<Atom> = args.iter().map(|a| self.atom(lets, &a.value)).collect();
                    self.push(
                        lets,
                        span,
                        Rhs::Apply {
                            callee: Atom::Var(f),
                            args,
                            arg_spans,
                            span,
                        },
                    )
                }
                Some(Res::Function { id, type_args }) => {
                    let receiver = self.atom(lets, target);
                    let receiver = (receiver, target.span);
                    self.call_function(lets, span, id, &type_args, Some(receiver), args)
                }
                _ => Atom::Unit,
            },
            ExprKind::Call { callee, args } => match self.res(span) {
                Some(Res::Function { id, type_args }) => {
                    self.call_function(lets, span, id, &type_args, None, args)
                }
                Some(Res::Ctor { kind, variant }) => {
                    let (args, arg_spans) = self.call_args(lets, span, args);
                    let ctor = self.ctor(kind, variant);
                    self.push(
                        lets,
                        span,
                        Rhs::Construct {
                            ctor,
                            args,
                            arg_spans,
                        },
                    )
                }
                Some(Res::Value) => {
                    let callee = self.atom(lets, callee);
                    let arg_spans: Vec<Span> = args.iter().map(|a| a.value.span).collect();
                    let args: Vec<Atom> = args.iter().map(|a| self.atom(lets, &a.value)).collect();
                    self.push(
                        lets,
                        span,
                        Rhs::Apply {
                            callee,
                            args,
                            arg_spans,
                            span,
                        },
                    )
                }
                _ => Atom::Unit,
            },
            ExprKind::CapOp { cap, op, args } => match self.res(span) {
                Some(Res::Op { capability, .. }) => {
                    let (values, arg_spans) = self.call_args(lets, span, args);
                    let labels: Vec<Label> = arg_spans
                        .iter()
                        .map(|arg| {
                            self.records
                                .types
                                .get(arg)
                                .map_or_else(Label::public, Ty::deep_label)
                        })
                        .collect();
                    let capability_path = self.capability_path(Some(capability));
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
                            capability_path,
                            op: self.name(op).to_owned(),
                            args: values,
                            arg_spans,
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
        if let Some(plain) = plain_text(text) {
            return Atom::Text(plain);
        }
        let mut parts = Vec::new();
        let mut part_spans = Vec::new();
        for part in &text.parts {
            match part {
                TextPart::Text(chunk) => {
                    parts.push(Atom::Text(chunk.clone()));
                    part_spans.push(span);
                }
                TextPart::Interpolation(expr) => {
                    let value = self.atom(lets, expr);
                    let t = self.temp();
                    let label = self
                        .records
                        .types
                        .get(&expr.span)
                        .map_or_else(Label::public, Ty::deep_label);
                    lets.push(Let::inferred(
                        t.clone(),
                        with_label(text_ty(), &label),
                        Bound::Rhs(Rhs::Prim {
                            prim: Prim::ToText,
                            args: vec![value],
                            arg_spans: vec![expr.span],
                            span: expr.span,
                        }),
                        expr.span,
                    ));
                    parts.push(Atom::Var(t));
                    part_spans.push(expr.span);
                }
            }
        }
        self.push(
            lets,
            span,
            Rhs::Prim {
                prim: Prim::Concat,
                args: parts,
                arg_spans: part_spans,
                span,
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
            let constant = CoreExpr::Atom {
                atom: Atom::Bool(constant.unwrap_or(false)),
                span,
            };
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
        let arg_spans = vec![left.span, right.span];
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
                arg_spans,
                span,
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
        receiver: Option<(Atom, Span)>,
        args: &[Arg],
    ) -> Atom {
        let Some(def) = self.env.functions.get(id) else {
            self.broken = true;
            return Atom::Unit;
        };
        let (values, arg_spans) = self.ordered_values(lets, span, receiver, args);
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
            let arg_span = arg_spans.first().copied().unwrap_or(span);
            return self.push(
                lets,
                span,
                Rhs::Declassify {
                    policy: self.function_path(id),
                    via,
                    arg,
                    arg_span,
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
                arg_spans,
                span,
            },
        )
    }

    /// The arguments of the call at `span`, evaluated in written order and placed in
    /// parameter order (`spec/04 §3` rule 5, `spec/09 §3`), with their sources.
    fn call_args(
        &mut self,
        lets: &mut Vec<Let>,
        span: Span,
        args: &[Arg],
    ) -> (Vec<Atom>, Vec<Span>) {
        self.ordered_values(lets, span, None, args)
    }

    fn ordered_values(
        &mut self,
        lets: &mut Vec<Let>,
        span: Span,
        receiver: Option<(Atom, Span)>,
        args: &[Arg],
    ) -> (Vec<Atom>, Vec<Span>) {
        let order = self.order(span);
        let mut written: Vec<(Atom, Span)> = receiver.into_iter().collect();
        for arg in args {
            let atom = self.atom(lets, &arg.value);
            written.push((atom, arg.value.span));
        }
        if written.len() != order.len() {
            self.broken = true;
        }
        let count = order.iter().flatten().max().map_or(0, |max| max + 1);
        let mut slots: Vec<Option<(Atom, Span)>> = vec![None; count.max(written.len())];
        for (value, target) in written.into_iter().zip(order.iter().copied()) {
            match target.and_then(|t| slots.get_mut(t)) {
                Some(slot) => *slot = Some(value),
                None => self.broken = true,
            }
        }
        slots.into_iter().flatten().unzip()
    }

    /// A function used as a value at `span`: `λ(x, ...). f[τ*](x, ...)`, whose parameters keep
    /// the declared names and types, refinements included (ADR 0037 rule 1).
    fn function_value(&mut self, id: usize, type_args: &[Ty], span: Span) -> Rhs {
        let Some(def) = self.env.functions.get(id) else {
            self.broken = true;
            return Rhs::Atom(Atom::Unit);
        };
        let mark = self.scope.len();
        let params: Vec<(Name, Type)> = def
            .params
            .iter()
            .map(|p| {
                let declared = Declared {
                    base: p.ty.base.substitute(type_args),
                    refinements: p.ty.refinements.clone(),
                };
                let ty = self.declared(&declared);
                (self.bind(p.name), ty)
            })
            .collect();
        self.scope.truncate(mark);
        let args: Vec<Atom> = params.iter().map(|(n, _)| Atom::Var(n.clone())).collect();
        let arg_spans = vec![span; args.len()];
        let t = self.temp();
        let result = self.declared(&Declared {
            base: def.result.base.substitute(type_args),
            refinements: def.result.refinements.clone(),
        });
        let body = CoreExpr::Let {
            name: t.clone(),
            ty: result,
            value: Box::new(Bound::Rhs(Rhs::Call {
                function: self.function_path(id),
                type_args: type_args.iter().map(|a| self.ty(a)).collect(),
                args,
                arg_spans,
                span,
            })),
            body: Box::new(CoreExpr::Atom {
                atom: Atom::Var(t),
                span,
            }),
            span,
            written: false,
        };
        Rhs::Lambda {
            params,
            body: Box::new(body),
            function: Some(self.function_path(id)),
        }
    }
}

// ----- helpers -----

fn close(lets: Vec<Let>, tail: CoreExpr) -> CoreExpr {
    lets.into_iter()
        .rev()
        .fold(tail, |body, binding| CoreExpr::Let {
            name: binding.name,
            ty: binding.ty,
            value: Box::new(binding.value),
            body: Box::new(body),
            span: binding.span,
            written: binding.written,
        })
}

/// Binds `name` to `expr`, flattening a `let` chain that ends in an atom into `lets`.
fn bind_expr(lets: &mut Vec<Let>, name: Name, ty: Type, expr: CoreExpr, span: Span, written: bool) {
    let mut current = expr;
    loop {
        match current {
            CoreExpr::Let {
                name: inner,
                ty: inner_ty,
                value,
                body,
                span: inner_span,
                written: inner_written,
            } => {
                lets.push(Let {
                    name: inner,
                    ty: inner_ty,
                    value: *value,
                    span: inner_span,
                    written: inner_written,
                });
                current = *body;
            }
            // ADR 0037 rule 2: an annotated binder whose function type differs from the type
            // of the value keeps the temporary, so the flow from one to the other is an
            // `Atom::Var` right side that the verifier checks, whatever the value is (N-005).
            CoreExpr::Atom {
                atom: Atom::Var(var),
                ..
            } if lets.last().is_some_and(|last| {
                last.name == var && !(written && last.ty != ty && has_fn_type(&ty))
            }) =>
            {
                if let Some(last) = lets.pop() {
                    lets.push(Let {
                        name,
                        ty,
                        value: last.value,
                        span,
                        written,
                    });
                }
                return;
            }
            CoreExpr::Atom { atom, .. } => {
                lets.push(Let {
                    name,
                    ty,
                    value: Bound::Rhs(Rhs::Atom(atom)),
                    span,
                    written,
                });
                return;
            }
            other => {
                lets.push(Let {
                    name,
                    ty,
                    value: Bound::Expr(other),
                    span,
                    written,
                });
                return;
            }
        }
    }
}

/// Whether a Core type is or contains a function type, under type arguments.
fn has_fn_type(ty: &Type) -> bool {
    match ty {
        Type::Fn { .. } => true,
        Type::Base {
            base: Base::Named { args, .. },
            ..
        } => args.iter().any(has_fn_type),
        Type::Base { .. } => false,
    }
}

/// The span of a block expression.
fn block_expr_span(expr: &BlockExpr) -> Span {
    match expr {
        BlockExpr::When(when) => when.span,
        BlockExpr::If(block) => block.span,
    }
}

/// The text of a literal without interpolation.
fn plain_text(text: &TextLit) -> Option<String> {
    let mut plain = String::new();
    for part in &text.parts {
        match part {
            TextPart::Text(chunk) => plain.push_str(chunk),
            TextPart::Interpolation(_) => return None,
        }
    }
    Some(plain)
}

/// The bounds of range sugar as the checker writes it: `in 0..5000` or `in 1..<20`.
fn parse_range(text: &str) -> Option<(String, String, bool)> {
    let range = text.strip_prefix("in ")?;
    if let Some((low, high)) = range.split_once("..<") {
        return Some((low.to_owned(), high.to_owned(), true));
    }
    let (low, high) = range.split_once("..")?;
    Some((low.to_owned(), high.to_owned(), false))
}

fn bin_op(op: BinaryOp) -> BinOp {
    match op {
        BinaryOp::Implies => BinOp::Implies,
        BinaryOp::Or => BinOp::Or,
        BinaryOp::And => BinOp::And,
        BinaryOp::Eq => BinOp::Eq,
        BinaryOp::NotEq => BinOp::Ne,
        BinaryOp::Lt => BinOp::Lt,
        BinaryOp::Le => BinOp::Le,
        BinaryOp::Gt => BinOp::Gt,
        BinaryOp::Ge => BinOp::Ge,
        BinaryOp::Add => BinOp::Add,
        BinaryOp::Sub => BinOp::Sub,
        BinaryOp::Mul => BinOp::Mul,
        BinaryOp::Div => BinOp::Div,
        BinaryOp::Mod => BinOp::Mod,
    }
}

/// The first component of the checker type `ty` that the Core IR type `core` lost: a label, the
/// `uses` or `fails` of a function type, or a type argument, at any position (ADR 0038 D1).
/// Unification variables, `Never` and the error type have no Core IR form and are skipped.
fn lost(ty: &Ty, core: &Type) -> Option<String> {
    let label = match core {
        Type::Base { label, .. } | Type::Fn { label, .. } => label,
    };
    if matches!(ty.unlabeled(), Ty::Var(_) | Ty::Never | Ty::Error) {
        return None;
    }
    if *label != ty.top_label() {
        return Some(format!("the label `{}` became `{label}`", ty.top_label()));
    }
    match (ty.unlabeled(), core) {
        (
            Ty::Fn(function),
            Type::Fn {
                params,
                result,
                uses,
                fails,
                ..
            },
        ) => {
            let same_uses = function.uses.len() == uses.len()
                && function
                    .uses
                    .iter()
                    .zip(uses)
                    .all(|(need, core)| need.name == core.cap && need.ops == core.ops);
            if !same_uses {
                return Some("the `uses` of a function type".to_owned());
            }
            if function.fails.is_some() != fails.is_some() {
                return Some("the `fails` of a function type".to_owned());
            }
            if params.len() != function.params.len() {
                return Some("a parameter of a function type".to_owned());
            }
            // ADR 0037 rule 11: every refinement of a parameter or the result is in Core IR.
            let dropped = |refinements: &[Refinement], core: &Type| {
                let kept = match core {
                    Type::Base { refinements, .. } => refinements.len(),
                    Type::Fn { .. } => 0,
                };
                kept < refinements.len()
            };
            if let Some(index) = (0..params.len()).find(|index| {
                params
                    .get(*index)
                    .is_some_and(|core| dropped(function.refinements.param(*index), core))
            }) {
                return Some(format!(
                    "a refinement of parameter {} of a function type",
                    index + 1
                ));
            }
            if dropped(&function.refinements.result, result) {
                return Some("a refinement of the result of a function type".to_owned());
            }
            function
                .params
                .iter()
                .zip(params)
                .find_map(|(ty, core)| lost(ty, core))
                .or_else(|| lost(&function.result, result))
        }
        (Ty::Fn(_), Type::Base { .. }) => Some("a function type".to_owned()),
        (
            Ty::List(element),
            Type::Base {
                base: Base::Named { args, .. },
                ..
            },
        ) => args.first().and_then(|core| lost(element, core)),
        (
            Ty::Kind(_, args),
            Type::Base {
                base: Base::Named { args: core, .. },
                ..
            },
        ) => {
            if args.len() != core.len() {
                return Some("a type argument".to_owned());
            }
            args.iter().zip(core).find_map(|(ty, core)| lost(ty, core))
        }
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn logging() -> FnUse {
        FnUse {
            name: "log".to_owned(),
            capability: Some(0),
            ops: vec!["write".to_owned()],
        }
    }

    fn core_fn(uses: Vec<Use>, fails: Option<String>, label: Label) -> Type {
        Type::Fn {
            params: vec![plain(Base::Text)],
            result: Box::new(unit()),
            uses,
            fails,
            label,
        }
    }

    #[test]
    fn the_consistency_check_finds_lost_uses_fails_and_labels() {
        let secret = Label {
            level: una_core::Level::Secret,
            categories: std::collections::BTreeSet::new(),
            symbols: std::collections::BTreeSet::new(),
        };
        let function = Ty::Fn(Box::new(super::super::ty::FnTy {
            params: vec![Ty::Text],
            result: Ty::Unit,
            uses: vec![logging()],
            fails: Some(Ty::Kind(0, Vec::new())),
            refinements: super::super::ty::FnRefinements::default(),
        }));
        let kept = core_fn(
            vec![Use {
                cap: "log".to_owned(),
                capability: "Log".to_owned(),
                ops: vec!["write".to_owned()],
            }],
            Some("Boom".to_owned()),
            Label::public(),
        );
        assert_eq!(lost(&function, &kept), None);
        let no_uses = core_fn(Vec::new(), Some("Boom".to_owned()), Label::public());
        assert!(lost(&function, &no_uses).is_some());
        let no_fails = core_fn(
            vec![Use {
                cap: "log".to_owned(),
                capability: "Log".to_owned(),
                ops: vec!["write".to_owned()],
            }],
            None,
            Label::public(),
        );
        assert!(lost(&function, &no_fails).is_some());
        let listed = Ty::List(Box::new(function.clone().with_label(&secret)));
        let dropped = Type::Base {
            base: Base::Named {
                module: "prelude".to_owned(),
                name: "List".to_owned(),
                args: vec![kept.clone()],
            },
            refinements: Vec::new(),
            label: Label::public(),
        };
        assert!(lost(&listed, &dropped).is_some());
    }

    #[test]
    fn the_consistency_check_finds_a_lost_refinement_of_a_function_type() {
        // ADR 0037 rule 11: `fn(Small) -> Pos` keeps both refinements.
        let alias = |id: usize| super::super::ty::Refinement::Alias(id, Vec::new());
        let function = Ty::Fn(Box::new(super::super::ty::FnTy {
            params: vec![Ty::Int],
            result: Ty::Int,
            uses: Vec::new(),
            fails: None,
            refinements: super::super::ty::FnRefinements {
                params: vec![vec![alias(0)]],
                result: vec![alias(1)],
            },
        }));
        let refined = |text: &str| Type::Base {
            base: Base::Int,
            refinements: vec![CoreRefinement {
                name: text.to_owned(),
                clause: CoreClause {
                    term: Term::Bool(true),
                    text: text.to_owned(),
                    span: None,
                },
            }],
            label: Label::public(),
        };
        let core = |param: Type, result: Type| Type::Fn {
            params: vec![param],
            result: Box::new(result),
            uses: Vec::new(),
            fails: None,
            label: Label::public(),
        };
        let kept = core(refined("Small"), refined("Pos"));
        assert_eq!(lost(&function, &kept), None);
        let no_param = core(plain(Base::Int), refined("Pos"));
        assert_eq!(
            lost(&function, &no_param).as_deref(),
            Some("a refinement of parameter 1 of a function type")
        );
        let no_result = core(refined("Small"), plain(Base::Int));
        assert_eq!(
            lost(&function, &no_result).as_deref(),
            Some("a refinement of the result of a function type")
        );
    }
}
