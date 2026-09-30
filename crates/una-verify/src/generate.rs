//! Obligation generation (`spec/12 §3`): a walk over Core IR that keeps the path context Π,
//! and builds the query of each obligation (`spec/12 §4`).

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use una_core::{
    Atom, Base, BinOp, Bound, CapOp, Clause, Ctor, DataVariant, Decl, Expr, Fun, Name, Prim,
    Program, Refinement, Rhs, Source, Term, Type, print_term_with,
};
use una_diag::ObligationKind;
use una_syntax::{SourceDb, Span};

use crate::smt::{
    Const, ConstRole, Definition, F, Fun as SmtFun, FunRole, HASHED, LIST, Query, Sort, TEXT, UNIT,
    kind_sort, path_selector,
};

/// A proof obligation of `spec/12 §3`, with its query.
#[derive(Clone, Debug)]
pub struct Obligation {
    /// The stable id, `ob-` and 12 hex digits (`spec/12 §8`).
    pub id: String,
    /// The kind.
    pub kind: ObligationKind,
    /// The module path.
    pub module: String,
    /// The enclosing declaration, qualified.
    pub declaration: String,
    /// The goal, as text.
    pub goal: String,
    /// The site of the obligation, or the clause of a return obligation (DECISION D-118).
    pub source: Span,
    /// The declaring clause, and what it is.
    pub related: Option<(Span, &'static str)>,
    /// For a return obligation, the returned expression of each path, in the order of the
    /// query's path selectors (DECISION D-113).
    pub paths: Vec<Span>,
    /// The hypotheses of the path context, as text.
    pub hypotheses: Vec<String>,
    /// The query: `Π ∧ ¬goal`.
    pub(crate) query: Query,
    /// The terms whose values a counterexample shows (DECISION D-119).
    pub(crate) shown: Vec<Shown>,
}

/// A term that a counterexample shows, by source name.
#[derive(Clone, Debug)]
pub(crate) struct Shown {
    pub(crate) name: String,
    pub(crate) term: F,
    pub(crate) show: Show,
}

/// How a value shows.
#[derive(Clone, Debug)]
pub(crate) enum Show {
    /// An `Int` or a `Bool`, as the solver writes it.
    Value,
    /// A value of a sum, by the variant its tag names.
    Variant { tag: F, names: Vec<String> },
    /// Another value, as `<Kind>#<n>`.
    Opaque(String),
}

/// The obligations of `program`, in the order of `spec/12 §5`: file, start offset, kind.
pub fn obligations(program: &Program, sources: &SourceDb) -> Vec<Obligation> {
    let index = Index::new(program);
    let mut all = Vec::new();
    for module in &program.modules {
        for decl in &module.decls {
            let mut walker = Walker::new(&index, sources, &module.path);
            match decl {
                Decl::Fun(fun) => walker.function(fun),
                Decl::Handler {
                    name,
                    params,
                    capability_path,
                    ops,
                    ..
                } => {
                    for op in ops {
                        walker.declaration = format!("{}.{name}.{}", module.path, op.name);
                        walker.handler_op(params, capability_path, op);
                    }
                }
                Decl::Pred {
                    name,
                    reflect: false,
                    params,
                    body,
                    ..
                } => {
                    walker.declaration = format!("{}.{name}", module.path);
                    walker.opaque(params, body);
                }
                _ => {}
            }
            all.extend(walker.obligations);
        }
        for example in &module.examples {
            let mut walker = Walker::new(&index, sources, &module.path);
            walker.declaration =
                format!("{}.example {}", module.path, una_core::quote(&example.name));
            let mut leaves = Vec::new();
            walker.walk(&example.body, &Ctx::default(), &mut leaves);
            // `spec/08 §2` rule 4: `expect` lines are verified too, with `result` the value of
            // one of the `do` line's paths.
            for expectation in &example.expectations {
                if let una_core::Expectation::Holds { result, check, .. } = expectation {
                    let mut ctx = Ctx::default();
                    if let Some(ty) = leaves.iter().find_map(|l| l.value.as_ref().map(|v| &v.ty)) {
                        ctx.types.insert(result.clone(), ty.clone());
                        let x = own(&ctx, result);
                        let paths: Vec<F> =
                            leaves.iter().map(|leaf| walker.path(leaf, 0, &x)).collect();
                        ctx.define(F::or(paths));
                    }
                    let mut discarded = Vec::new();
                    walker.walk(check, &ctx, &mut discarded);
                }
            }
            all.extend(walker.obligations);
        }
    }
    all.sort_by_key(|o| (o.source.file, o.source.start, o.kind));
    all
}

/// For each `ensures` clause of a public function, a query without any hypothesis, its
/// parameters and `result` unconstrained: a clause the solver proves this way is trivial, a
/// missing contract (`spec/13 §6`, DECISION D-123).
pub fn trivial_ensures(program: &Program, sources: &SourceDb) -> Vec<Obligation> {
    let index = Index::new(program);
    let mut all = Vec::new();
    for module in &program.modules {
        for decl in &module.decls {
            let Decl::Fun(fun) = decl else {
                continue;
            };
            if !fun.public || fun.ensures.is_empty() {
                continue;
            }
            let mut walker = Walker::new(&index, sources, &module.path);
            walker.declaration = format!("{}.{}", module.path, fun.name);
            let mut ctx = Ctx::default();
            for (name, ty) in &fun.params {
                ctx.types.insert(name.clone(), ty.clone());
            }
            ctx.types.insert("result".to_owned(), fun.result.clone());
            let result = own(&ctx, "result");
            for clause in &fun.ensures {
                let goal = {
                    let vars = |name: &str| Some(own(&ctx, name));
                    let env = Env {
                        vars: &vars,
                        it: None,
                        result: Some(result.clone()),
                    };
                    walker.term(&clause.term, &env)
                };
                walker.site(
                    ObligationKind::Postcondition,
                    &ctx,
                    goal.map(|g| g.f),
                    clause.text.clone(),
                    clause.span.unwrap_or(fun.span),
                    None,
                );
            }
            all.extend(walker.obligations);
        }
    }
    all
}

// ----- declarations -----

/// The contract of a callee: a function of the package or of the standard library.
struct Contract<'p> {
    params: &'p [(Name, Type)],
    result: &'p Type,
    requires: &'p [Clause],
    ensures: &'p [Clause],
}

/// A predicate: its parameters, and the definition of a reflect one.
type Predicate<'p> = (&'p [(Name, Type)], Option<&'p Term>);

/// A kind's variants, with its type parameters.
struct Kind<'p> {
    params: &'p [String],
    variants: &'p [DataVariant],
    record: bool,
}

/// The declarations of a program that obligations refer to.
pub(crate) struct Index<'p> {
    contracts: BTreeMap<(&'p str, &'p str), Contract<'p>>,
    predicates: BTreeMap<(&'p str, &'p str), Predicate<'p>>,
    /// Capabilities by qualified identity (`spec/10 §11` rule 8), the host ones included.
    capabilities: BTreeMap<(&'p str, &'p str), &'p [CapOp]>,
    handlers: BTreeMap<(&'p str, &'p str), &'p [(Name, Type)]>,
    kinds: BTreeMap<(&'p str, &'p str), Kind<'p>>,
    policies: BTreeMap<(&'p str, &'p str), (&'p Type, &'p Type)>,
    /// The `to` type of each policy, by its `via` function.
    vias: BTreeMap<(&'p str, &'p str), Vec<&'p Type>>,
}

/// Whether `module` is the prelude's (`spec/14 §1`). The prelude is identified by its module
/// path, which no module of a package that reaches the verifier can have: a user module path
/// whose first segment is `prelude` is U1009 at tier 0 (ADR 0038 D7), and tier 1 runs only
/// when tier 0 reports nothing (DECISION D-112). DECISION D-135: the reserved path is the
/// prelude's identity, rather than a new Core IR path that no source can spell, and a kind the
/// package declares is never taken for a prelude kind ([`Index::prelude_kind`]).
fn is_prelude(module: &str) -> bool {
    module == una_core::PRELUDE_MODULE
}

/// The variants of the prelude's kinds, which the program does not declare: names and field
/// counts (`spec/14 §1, §3`).
fn prelude_variants(kind: &str) -> Option<&'static [(&'static str, &'static [&'static str])]> {
    match kind {
        "Option" => Some(&[("Some", &["value"]), ("None", &[])]),
        "Result" => Some(&[("Ok", &["value"]), ("Err", &["failure"])]),
        "FileError" => Some(&[("NotFound", &[]), ("Denied", &[]), ("Io", &[])]),
        "KeyError" => Some(&[("UnknownKey", &[])]),
        _ => None,
    }
}

impl<'p> Index<'p> {
    fn new(program: &'p Program) -> Index<'p> {
        let mut index = Index {
            contracts: BTreeMap::new(),
            predicates: BTreeMap::new(),
            capabilities: BTreeMap::new(),
            handlers: BTreeMap::new(),
            kinds: BTreeMap::new(),
            policies: BTreeMap::new(),
            vias: BTreeMap::new(),
        };
        for signature in &program.library {
            index.contracts.insert(
                (signature.path.module.as_str(), signature.path.name.as_str()),
                Contract {
                    params: &signature.params,
                    result: &signature.result,
                    requires: &signature.requires,
                    ensures: &signature.ensures,
                },
            );
        }
        // `spec/14 §3`: the host capabilities, declared in `std.host`.
        for decl in &program.host {
            if let Decl::Cap { name, ops } = decl {
                index
                    .capabilities
                    .insert((una_core::HOST_MODULE, name.as_str()), ops);
            }
        }
        for module in &program.modules {
            let path = module.path.as_str();
            for decl in &module.decls {
                match decl {
                    Decl::Fun(fun) => {
                        index.contracts.insert(
                            (path, fun.name.as_str()),
                            Contract {
                                params: &fun.params,
                                result: &fun.result,
                                requires: &fun.requires,
                                ensures: &fun.ensures,
                            },
                        );
                    }
                    Decl::Pred {
                        name,
                        params,
                        formula,
                        ..
                    } => {
                        index
                            .predicates
                            .insert((path, name.as_str()), (params, formula.as_ref()));
                    }
                    Decl::Cap { name, ops } => {
                        index.capabilities.insert((path, name.as_str()), ops);
                    }
                    Decl::Handler { name, params, .. } => {
                        index.handlers.insert((path, name.as_str()), params);
                    }
                    Decl::Data {
                        name,
                        params,
                        variants,
                        record,
                    } => {
                        index.kinds.insert(
                            (path, name.as_str()),
                            Kind {
                                params,
                                variants,
                                record: *record,
                            },
                        );
                    }
                    Decl::Policy {
                        name,
                        from,
                        to,
                        via,
                    } => {
                        index.policies.insert((path, name.as_str()), (from, to));
                        index
                            .vias
                            .entry((via.module.as_str(), via.name.as_str()))
                            .or_default()
                            .push(to);
                    }
                    Decl::Alias { .. } => {}
                }
            }
        }
        index
    }

    /// The operation `op` of the capability declared as `capability`, looked up by resolved
    /// identity, never by bare name (ADR 0038 D6, `spec/10 §11` rule 8).
    fn operation(&self, capability: &una_core::Path, op: &str) -> Option<&'p CapOp> {
        self.capabilities
            .get(&(capability.module.as_str(), capability.name.as_str()))
            .and_then(|ops| ops.iter().find(|o| o.name == op))
    }

    /// The refinements of the `to` types of the policies whose `via` function is `name` of
    /// `module`, except those of `result`, the via function's own result type, which its own
    /// result obligations prove already.
    fn via_refinements(&self, module: &str, name: &str, result: &Type) -> Vec<&'p Refinement> {
        let own = refinements(result);
        let mut out: Vec<&'p Refinement> = Vec::new();
        for to in self.vias.get(&(module, name)).into_iter().flatten() {
            for refinement in refinements(to) {
                if !own.contains(refinement) && !out.contains(&refinement) {
                    out.push(refinement);
                }
            }
        }
        out
    }

    /// Whether the kind at (`module`, `name`) is a kind of the prelude. A kind the package
    /// declares never is, whatever its module is called.
    fn prelude_kind(&self, module: &str, name: &str) -> bool {
        is_prelude(module) && !self.kinds.contains_key(&(module, name))
    }

    /// The number of variants of the kind at (`module`, `name`), and their names.
    fn variant_names(&self, module: &str, name: &str) -> Vec<String> {
        if self.prelude_kind(module, name)
            && let Some(variants) = prelude_variants(name)
        {
            return variants.iter().map(|(v, _)| (*v).to_owned()).collect();
        }
        self.kinds
            .get(&(module, name))
            .map(|kind| kind.variants.iter().map(|v| v.name.clone()).collect())
            .unwrap_or_default()
    }

    /// Whether the kind is a sum whose variants have no fields, so that equality and order
    /// are those of its tags (`spec/03 §1`).
    fn payload_free(&self, module: &str, name: &str) -> bool {
        if self.prelude_kind(module, name) {
            return prelude_variants(name)
                .is_some_and(|variants| variants.iter().all(|(_, fields)| fields.is_empty()));
        }
        self.kinds
            .get(&(module, name))
            .is_some_and(|kind| !kind.record && kind.variants.iter().all(|v| v.fields.is_empty()))
    }

    /// The declared type of field `index` of `variant` of the kind of `ty`, with the kind's
    /// type parameters replaced by its arguments.
    fn field_type(&self, ty: &Type, variant: usize, index: usize) -> Option<Type> {
        let Type::Base {
            base: Base::Named { module, name, args },
            ..
        } = ty
        else {
            return None;
        };
        if self.prelude_kind(module, name) {
            // `Some(value: T)`, `Ok(value: T)`, `Err(failure: E)`.
            return match (name.as_str(), variant) {
                ("Option" | "Result", 0) | ("Result", 1) if index == 0 => {
                    args.get(variant).cloned()
                }
                _ => None,
            };
        }
        let kind = self.kinds.get(&(module.as_str(), name.as_str()))?;
        let (_, field) = kind.variants.get(variant)?.fields.get(index)?;
        Some(substitute(field, kind.params, args))
    }
}

/// `ty` with the type parameters `params` replaced by `args`.
fn substitute(ty: &Type, params: &[String], args: &[Type]) -> Type {
    match ty {
        Type::Base {
            base: Base::Param(name),
            refinements,
            label,
        } => match params
            .iter()
            .position(|p| p == name)
            .and_then(|i| args.get(i))
        {
            Some(Type::Base {
                base,
                refinements: own,
                ..
            }) => Type::Base {
                base: base.clone(),
                refinements: own.iter().chain(refinements).cloned().collect(),
                label: label.clone(),
            },
            Some(other) => other.clone(),
            None => ty.clone(),
        },
        Type::Base {
            base:
                Base::Named {
                    module,
                    name,
                    args: own,
                },
            refinements,
            label,
        } => Type::Base {
            base: Base::Named {
                module: module.clone(),
                name: name.clone(),
                args: own.iter().map(|a| substitute(a, params, args)).collect(),
            },
            refinements: refinements.clone(),
            label: label.clone(),
        },
        other => other.clone(),
    }
}

/// The refinements of a type (`spec/03 §2` rule 3).
fn refinements(ty: &Type) -> &[Refinement] {
    match ty {
        Type::Base { refinements, .. } => refinements,
        Type::Fn { .. } => &[],
    }
}

fn base_type(base: Base) -> Type {
    Type::Base {
        base,
        refinements: Vec::new(),
        label: una_core::Label::public(),
    }
}

/// The sort of a Una type (`spec/12 §4`, DECISION D-115), in the namespaces of ADR 0038 D8:
/// `Int` and `Bool` are SMT-LIB's, the other built-in types and the kinds of the prelude are
/// the encoder's (`i!Text`, `i!List_<T>`, `i!Option_<T>`, `i!FileError`), and the kinds and
/// type parameters of the program are the user's (`u!Kind_<module>_<Name>`, `u!Param_<name>`).
pub(crate) fn sort(ty: &Type) -> Sort {
    match ty {
        Type::Fn { .. } => "i!Fn".to_owned(),
        Type::Base { base, .. } => match base {
            Base::Int => "Int".to_owned(),
            Base::Bool => "Bool".to_owned(),
            Base::Text => TEXT.to_owned(),
            Base::Unit => UNIT.to_owned(),
            Base::Hashed => HASHED.to_owned(),
            Base::Named { module, name, args } => {
                let args: Vec<Sort> = args.iter().map(sort).collect();
                if is_prelude(module) {
                    let mut sort = format!("i!{name}");
                    for arg in &args {
                        sort.push('_');
                        sort.push_str(arg);
                    }
                    sort
                } else {
                    kind_sort(module, name, &args)
                }
            }
            Base::Param(name) => format!("u!Param_{name}"),
            Base::Unknown => "i!Unknown".to_owned(),
        },
    }
}

/// How the values of a type show in a counterexample that cannot print them, `<name>#<n>`
/// (`spec/12 §7`): by the type's source name, never its sort.
fn type_name(ty: &Type) -> String {
    match ty {
        Type::Fn { .. } => "Fn".to_owned(),
        Type::Base { base, .. } => match base {
            Base::Int => "Int".to_owned(),
            Base::Bool => "Bool".to_owned(),
            Base::Text => "Text".to_owned(),
            Base::Unit => "Unit".to_owned(),
            Base::Hashed => "Hashed".to_owned(),
            Base::Named { name, .. } | Base::Param(name) => name.clone(),
            Base::Unknown => "Unknown".to_owned(),
        },
    }
}

// ----- the walk -----

/// A value with its Una type, whose sort and fields the encoding needs.
#[derive(Clone, Debug)]
struct Tv {
    f: F,
    ty: Type,
}

/// A hypothesis of the path context; a hidden one defines a temporary and is not listed.
#[derive(Clone, Debug)]
struct Hyp {
    f: F,
    shown: bool,
}

/// The path context Π and the types of the binders in scope.
#[derive(Clone, Debug, Default)]
struct Ctx {
    hyps: Vec<Hyp>,
    types: BTreeMap<Name, Type>,
}

impl Ctx {
    fn assume(&mut self, f: F) {
        self.hyps.push(Hyp { f, shown: true });
    }

    fn define(&mut self, f: F) {
        self.hyps.push(Hyp { f, shown: false });
    }
}

/// A normal return path: its context, the value it returns and where it is written.
#[derive(Clone, Debug)]
struct Leaf {
    hyps: Vec<Hyp>,
    value: Option<Tv>,
    span: Span,
    /// For the value of an `attempt`, the normal returns of the attempted call.
    returns: Vec<Leaf>,
}

/// How the terms of a clause are replaced: its binders, `it` and `result`.
struct Env<'a> {
    vars: &'a dyn Fn(&str) -> Option<Tv>,
    it: Option<Tv>,
    result: Option<Tv>,
}

struct Walker<'p, 'i> {
    index: &'i Index<'p>,
    sources: &'i SourceDb,
    module: &'p str,
    declaration: String,
    obligations: Vec<Obligation>,
    /// The definitions of the reflect predicates met so far.
    definitions: BTreeMap<String, Definition>,
    /// How each temporary shows in listed hypotheses.
    temps: BTreeMap<String, String>,
    /// The term each temporary is defined as, where it is one: how it shows.
    defs: BTreeMap<String, F>,
}

impl<'p, 'i> Walker<'p, 'i> {
    fn new(index: &'i Index<'p>, sources: &'i SourceDb, module: &'p str) -> Self {
        Walker {
            index,
            sources,
            module,
            declaration: String::new(),
            obligations: Vec::new(),
            definitions: BTreeMap::new(),
            temps: BTreeMap::new(),
            defs: BTreeMap::new(),
        }
    }

    fn text(&self, span: Span) -> String {
        self.sources
            .file(span.file)
            .text()
            .get(span.start..span.end)
            .unwrap_or("")
            .to_owned()
    }

    // ----- declarations -----

    fn function(&mut self, fun: &'p Fun) {
        self.declaration = format!("{}.{}", self.module, fun.name);
        let mut ctx = Ctx::default();
        for (name, ty) in &fun.params {
            self.bind(&mut ctx, name, ty);
        }
        for clause in &fun.requires {
            let vars = |name: &str| Some(own(&ctx, name));
            let env = Env {
                vars: &vars,
                it: None,
                result: None,
            };
            if let Some(f) = self.term(&clause.term, &env) {
                ctx.assume(f.f);
            }
        }
        let mut leaves = Vec::new();
        self.walk(&fun.body, &ctx, &mut leaves);
        let result = Tv {
            f: constant("result", &fun.result),
            ty: fun.result.clone(),
        };
        for refinement in refinements(&fun.result) {
            self.result_refinement(&ctx, &leaves, &result, refinement, fun.span);
        }
        // ADR 0038 D5: the result of a policy's `via` function satisfies the refinement of the
        // policy's `to` type, which T-Declassify assumes (`spec/12 §3`, `spec/07 §5` rule 3).
        for refinement in self
            .index
            .via_refinements(self.module, &fun.name, &fun.result)
        {
            self.result_refinement(&ctx, &leaves, &result, refinement, fun.span);
        }
        for clause in &fun.ensures {
            let goal = {
                let vars = |name: &str| Some(own(&ctx, name));
                let env = Env {
                    vars: &vars,
                    it: None,
                    result: Some(result.clone()),
                };
                self.term(&clause.term, &env)
            };
            let source = clause.span.unwrap_or(fun.span);
            self.returns(
                ObligationKind::Postcondition,
                &ctx,
                &leaves,
                &result,
                goal,
                clause.text.clone(),
                source,
                clause.span.map(|span| (span, "ensures clause")),
            );
        }
    }

    /// A `refinement` obligation over every return path: `result` satisfies `refinement`.
    fn result_refinement(
        &mut self,
        ctx: &Ctx,
        leaves: &[Leaf],
        result: &Tv,
        refinement: &Refinement,
        source: Span,
    ) {
        let goal = {
            let vars = |name: &str| Some(own(ctx, name));
            let env = Env {
                vars: &vars,
                it: Some(result.clone()),
                result: Some(result.clone()),
            };
            self.term(&refinement.clause.term, &env)
        };
        let text = print_term_with(&refinement.clause.term, &|term| {
            matches!(term, Term::It).then(|| "result".to_owned())
        });
        self.returns(
            ObligationKind::Refinement,
            ctx,
            leaves,
            result,
            goal,
            text,
            source,
            refinement
                .clause
                .span
                .map(|span| (span, clause_role(refinement))),
        );
    }

    /// An operation of a handler, checked against the contract of its capability's operation
    /// (`spec/10 §8`).
    fn handler_op(
        &mut self,
        handler_params: &'p [(Name, Type)],
        capability: &una_core::Path,
        op: &'p una_core::HandlerOp,
    ) {
        let mut ctx = Ctx::default();
        for (name, ty) in handler_params {
            self.bind(&mut ctx, name, ty);
        }
        let signature = self.index.operation(capability, &op.name);
        let mut renames: BTreeMap<&str, &str> = BTreeMap::new();
        if let Some(signature) = signature {
            for ((name, ty), binder) in signature.params.iter().zip(&op.params) {
                self.bind(&mut ctx, binder, ty);
                renames.insert(name.as_str(), binder.as_str());
            }
            for clause in &signature.requires {
                let vars = |name: &str| renames.get(name).map(|binder| own(&ctx, binder));
                let env = Env {
                    vars: &vars,
                    it: None,
                    result: None,
                };
                if let Some(f) = self.term(&clause.term, &env) {
                    ctx.assume(f.f);
                }
            }
        }
        let mut leaves = Vec::new();
        self.walk(&op.body, &ctx, &mut leaves);
        let Some(signature) = signature else {
            return;
        };
        let result = Tv {
            f: constant("result", &signature.result),
            ty: signature.result.clone(),
        };
        let Some(span) = leaves.first().map(|leaf| leaf.span) else {
            return;
        };
        for refinement in refinements(&signature.result) {
            let goal = {
                let env = Env {
                    vars: &|_| None,
                    it: Some(result.clone()),
                    result: Some(result.clone()),
                };
                self.term(&refinement.clause.term, &env)
            };
            let text = print_term_with(&refinement.clause.term, &|term| {
                matches!(term, Term::It).then(|| "result".to_owned())
            });
            self.returns(
                ObligationKind::Refinement,
                &ctx,
                &leaves,
                &result,
                goal,
                text,
                span,
                refinement
                    .clause
                    .span
                    .map(|span| (span, clause_role(refinement))),
            );
        }
        for clause in &signature.ensures {
            let goal = {
                let vars = |name: &str| renames.get(name).map(|binder| own(&ctx, binder));
                let env = Env {
                    vars: &vars,
                    it: None,
                    result: Some(result.clone()),
                };
                self.term(&clause.term, &env)
            };
            self.returns(
                ObligationKind::Postcondition,
                &ctx,
                &leaves,
                &result,
                goal,
                clause.text.clone(),
                clause.span.unwrap_or(span),
                clause.span.map(|span| (span, "ensures clause")),
            );
        }
    }

    /// The body of an opaque predicate: code, whose obligations are generated as usual.
    fn opaque(&mut self, params: &'p [(Name, Type)], body: &'p Expr) {
        let mut ctx = Ctx::default();
        for (name, ty) in params {
            self.bind(&mut ctx, name, ty);
        }
        let mut leaves = Vec::new();
        self.walk(body, &ctx, &mut leaves);
    }

    /// Brings a binder into scope with the refinements of its type as hypotheses.
    fn bind(&mut self, ctx: &mut Ctx, name: &str, ty: &Type) {
        ctx.types.insert(name.to_owned(), ty.clone());
        let value = own(ctx, name);
        self.assume_refinements(ctx, &value);
    }

    fn assume_refinements(&mut self, ctx: &mut Ctx, value: &Tv) {
        for refinement in refinements(&value.ty) {
            let env = Env {
                vars: &|_| None,
                it: Some(value.clone()),
                result: None,
            };
            if let Some(f) = self.term(&refinement.clause.term, &env) {
                ctx.assume(f.f);
            }
        }
    }

    // ----- expressions -----

    fn walk(&mut self, expr: &'p Expr, start: &Ctx, leaves: &mut Vec<Leaf>) {
        let mut ctx = start.clone();
        let mut expr = expr;
        loop {
            match expr {
                Expr::Atom { atom, span } => {
                    leaves.push(Leaf {
                        hyps: ctx.hyps.clone(),
                        value: Some(self.atom(atom, &ctx)),
                        span: *span,
                        returns: Vec::new(),
                    });
                    return;
                }
                Expr::Unreachable { span, goal } => {
                    // DECISION D-111: the path after the last arm is infeasible.
                    self.site(
                        ObligationKind::Exhaustive,
                        &ctx,
                        Some(F::Bool(false)),
                        goal.clone(),
                        *span,
                        None,
                    );
                    return;
                }
                Expr::Let {
                    name,
                    ty,
                    value,
                    body,
                    span,
                    written,
                } => {
                    match value.as_ref() {
                        Bound::Rhs(rhs) => self.rhs(name, ty, rhs, &mut ctx),
                        Bound::Expr(inner) => {
                            let mut inner_leaves = Vec::new();
                            self.walk(inner, &ctx, &mut inner_leaves);
                            ctx.types.insert(name.clone(), ty.clone());
                            let x = own(&ctx, name);
                            // `spec/03 §7`: a value in the refinement language is selfified;
                            // `and`, `or`, `implies`, `in` and `is` are branches in Core.
                            let mut locals = BTreeMap::new();
                            if let Some(value) = self.pure(inner, &ctx, &mut locals)
                                && value.sort() == x.f.sort()
                            {
                                ctx.define(F::eq(x.f.clone(), value.clone()));
                                if name.starts_with('%') {
                                    self.defs.insert(name.clone(), value);
                                }
                            } else if !inner_leaves.is_empty() {
                                // The binder is the value of one of the paths.
                                let base = ctx.hyps.len();
                                let paths: Vec<F> = inner_leaves
                                    .iter()
                                    .map(|leaf| self.path(leaf, base, &x))
                                    .collect();
                                ctx.define(F::or(paths));
                            }
                        }
                    }
                    if name.starts_with('%') && !self.temps.contains_key(name.as_str()) {
                        let text = self.text(*span);
                        self.temps.insert(name.clone(), text);
                    }
                    let x = own(&ctx, name);
                    if *written {
                        let value_text = self.text(*span);
                        for refinement in refinements(ty) {
                            let env = Env {
                                vars: &|_| None,
                                it: Some(x.clone()),
                                result: None,
                            };
                            let goal = self.term(&refinement.clause.term, &env);
                            let text = instantiate(&refinement.clause.term, &value_text);
                            self.site(
                                ObligationKind::Refinement,
                                &ctx,
                                goal.map(|g| g.f),
                                text,
                                *span,
                                refinement
                                    .clause
                                    .span
                                    .map(|span| (span, clause_role(refinement))),
                            );
                        }
                    }
                    self.assume_refinements(&mut ctx, &x);
                    expr = body;
                }
                Expr::If {
                    cond,
                    then,
                    otherwise,
                } => {
                    let condition = self.atom(cond, &ctx).f;
                    let mut then_ctx = ctx.clone();
                    then_ctx.assume(condition.clone());
                    self.walk(then, &then_ctx, leaves);
                    ctx.assume(F::not(condition));
                    expr = otherwise;
                }
                Expr::Case {
                    scrutinee,
                    arms,
                    otherwise,
                } => {
                    let value = self.atom(scrutinee, &ctx);
                    for arm in arms {
                        let mut arm_ctx = ctx.clone();
                        if let Some(tag) = self.tag(&value) {
                            arm_ctx.assume(F::eq(tag, F::Int(arm.ctor.index.to_string())));
                        }
                        for (index, bind) in arm.binds.iter().enumerate() {
                            let Some(bind) = bind else {
                                continue;
                            };
                            let Some(field_ty) =
                                self.index.field_type(&value.ty, arm.ctor.index, index)
                            else {
                                continue;
                            };
                            arm_ctx.types.insert(bind.clone(), field_ty.clone());
                            let x = own(&arm_ctx, bind);
                            if let Some(field) = self.field(&value, arm.ctor.index, index) {
                                arm_ctx.define(F::eq(x.f.clone(), field.f));
                            }
                            self.assume_refinements(&mut arm_ctx, &x);
                        }
                        self.walk(&arm.body, &arm_ctx, leaves);
                    }
                    let Some(otherwise) = otherwise else {
                        return;
                    };
                    if let Some(tag) = self.tag(&value) {
                        for arm in arms {
                            ctx.assume(F::not(F::eq(
                                tag.clone(),
                                F::Int(arm.ctor.index.to_string()),
                            )));
                        }
                    }
                    expr = otherwise;
                }
                Expr::Fail {
                    ctor,
                    args,
                    arg_spans,
                    ..
                } => {
                    self.fields(ctor, args, arg_spans, &ctx);
                    return;
                }
                Expr::Handle {
                    body,
                    error: _,
                    handler,
                } => {
                    self.walk(body, &ctx, leaves);
                    expr = handler;
                }
                Expr::Attempt { body, span } => {
                    let mut returns = Vec::new();
                    self.walk(body, &ctx, &mut returns);
                    leaves.push(Leaf {
                        hyps: ctx.hyps.clone(),
                        value: None,
                        span: *span,
                        returns,
                    });
                    return;
                }
                Expr::With { source, body, .. } => {
                    if let Source::Handler {
                        handler,
                        args,
                        arg_spans,
                        ..
                    } = source
                        && let Some(params) = self
                            .index
                            .handlers
                            .get(&(handler.module.as_str(), handler.name.as_str()))
                    {
                        self.arguments(params, args, arg_spans, &ctx);
                    }
                    expr = body;
                }
            }
        }
    }

    /// `x is Ok implies ...` for the value `x` of an `attempt`: `Ok` is a normal return of the
    /// attempted call (`spec/06 §5`), so the facts of one of its return paths hold, among them
    /// the callee's `ensures`, and `x.value` is its value; `Err` carries nothing, as failures
    /// have no postcondition (`spec/04 §2`). DECISION D-131.
    fn attempted(&self, x: &Tv, leaf: &Leaf) -> Option<F> {
        if leaf.returns.is_empty() {
            return None;
        }
        let tag = self.tag(x)?;
        let payload = self.field(x, 0, 0)?;
        let base = leaf.hyps.len();
        let paths: Vec<F> = leaf
            .returns
            .iter()
            .map(|path| self.path(path, base, &payload))
            .collect();
        Some(F::op(
            "=>",
            vec![F::eq(tag, F::Int("0".to_owned())), F::or(paths)],
        ))
    }

    /// One path of an expression as a formula: its facts after the first `base` hypotheses,
    /// and `x` equal to its value, or for an `attempt`, to what the call returned.
    fn path(&self, leaf: &Leaf, base: usize, x: &Tv) -> F {
        let mut parts: Vec<F> = leaf.hyps.iter().skip(base).map(|h| h.f.clone()).collect();
        if let Some(value) = &leaf.value
            && value.f.sort() == x.f.sort()
        {
            parts.push(F::eq(x.f.clone(), value.f.clone()));
        }
        if let Some(ok) = self.attempted(x, leaf) {
            parts.push(ok);
        }
        F::and(parts)
    }

    /// A pure expression of Core as one term, when it is one: atoms, primitives and fields
    /// bound by `let`, `if`, and `case` on a single variant without bindings, which is how
    /// `and`, `or`, `implies`, `in` and `is` elaborate.
    fn pure(&mut self, expr: &Expr, ctx: &Ctx, locals: &mut BTreeMap<String, Tv>) -> Option<F> {
        let value = |walker: &mut Self, atom: &Atom, locals: &BTreeMap<String, Tv>| match atom {
            Atom::Var(name) => locals.get(name).cloned().unwrap_or_else(|| own(ctx, name)),
            other => walker.atom(other, ctx),
        };
        match expr {
            Expr::Atom { atom, .. } => Some(value(self, atom, locals).f),
            Expr::Let {
                name,
                ty,
                value: bound,
                body,
                ..
            } => {
                let Bound::Rhs(rhs) = bound.as_ref() else {
                    return None;
                };
                let bound_value = match rhs {
                    Rhs::Atom(atom) => value(self, atom, locals),
                    Rhs::Prim { prim, args, .. } => {
                        let values: Vec<Tv> = args.iter().map(|a| value(self, a, locals)).collect();
                        Tv {
                            f: self.prim(*prim, &values)?,
                            ty: ty.clone(),
                        }
                    }
                    Rhs::Field {
                        value: record,
                        index,
                        ..
                    } => {
                        let record = value(self, record, locals);
                        self.field(&record, 0, *index)?
                    }
                    _ => return None,
                };
                locals.insert(name.clone(), bound_value);
                self.pure(body, ctx, locals)
            }
            Expr::If {
                cond,
                then,
                otherwise,
            } => {
                let condition = value(self, cond, locals).f;
                let then = self.pure(then, ctx, locals)?;
                let otherwise = self.pure(otherwise, ctx, locals)?;
                Some(F::op("ite", vec![condition, then, otherwise]))
            }
            Expr::Case {
                scrutinee,
                arms,
                otherwise: Some(otherwise),
            } => {
                let [arm] = arms.as_slice() else {
                    return None;
                };
                if arm.binds.iter().any(Option::is_some) {
                    return None;
                }
                let scrutinee = value(self, scrutinee, locals);
                let test = F::eq(self.tag(&scrutinee)?, F::Int(arm.ctor.index.to_string()));
                let then = self.pure(&arm.body, ctx, locals)?;
                let otherwise = self.pure(otherwise, ctx, locals)?;
                Some(F::op("ite", vec![test, then, otherwise]))
            }
            _ => None,
        }
    }

    /// `let x: ty = rhs`: the obligations of the right-hand side, then its facts.
    fn rhs(&mut self, name: &str, ty: &Type, rhs: &'p Rhs, ctx: &mut Ctx) {
        ctx.types.insert(name.to_owned(), ty.clone());
        let x = own(ctx, name);
        match rhs {
            Rhs::Atom(atom) => {
                let value = self.atom(atom, ctx);
                if value.f.sort() == x.f.sort() {
                    ctx.define(F::eq(x.f.clone(), value.f.clone()));
                    if name.starts_with('%') {
                        self.defs.insert(name.to_owned(), value.f);
                    }
                }
                self.show_temp(name, atom);
            }
            Rhs::Prim {
                prim,
                args,
                arg_spans,
                span,
            } => {
                let values: Vec<Tv> = args.iter().map(|a| self.atom(a, ctx)).collect();
                if matches!(prim, Prim::Div | Prim::Mod)
                    && let (Some(divisor), Some(divisor_atom)) = (values.get(1), args.get(1))
                    && !matches!(divisor_atom, Atom::Int(digits) if digits.trim_start_matches('-') != "0")
                {
                    let text = format!(
                        "{} != 0",
                        arg_spans.get(1).map_or_else(String::new, |s| self.text(*s))
                    );
                    self.site(
                        ObligationKind::Division,
                        ctx,
                        Some(F::not(F::eq(divisor.f.clone(), F::Int("0".to_owned())))),
                        text,
                        *span,
                        None,
                    );
                }
                if let Some(value) = self.prim(*prim, &values)
                    && value.sort() == x.f.sort()
                {
                    ctx.define(F::eq(x.f.clone(), value.clone()));
                    if name.starts_with('%') {
                        self.defs.insert(name.to_owned(), value);
                    }
                    let texts: Vec<String> = arg_spans.iter().map(|s| self.text(*s)).collect();
                    if name.starts_with('%') {
                        self.temps
                            .insert(name.to_owned(), prim_text(*prim, &texts, &self.text(*span)));
                    }
                }
            }
            Rhs::Call {
                function,
                args,
                arg_spans,
                span,
                ..
            } => {
                let values: Vec<Tv> = args.iter().map(|a| self.atom(a, ctx)).collect();
                let key = (function.module.as_str(), function.name.as_str());
                if let Some(contract) = self.index.contracts.get(&key) {
                    self.call(contract, &values, arg_spans, *span, &x, ctx);
                } else if let Some((params, formula)) = self.index.predicates.get(&key) {
                    self.arguments(params, args, arg_spans, ctx);
                    let predicate = self.predicate(function, params, *formula, &values);
                    if let Some(predicate) = predicate {
                        ctx.define(F::eq(x.f.clone(), predicate));
                    }
                }
            }
            Rhs::Apply { .. } => {}
            Rhs::Op {
                capability_path,
                op,
                args,
                arg_spans,
                span,
                ..
            } => {
                let values: Vec<Tv> = args.iter().map(|a| self.atom(a, ctx)).collect();
                let signature = self.index.operation(capability_path, op);
                if let Some(signature) = signature {
                    let contract = Contract {
                        params: &signature.params,
                        result: &signature.result,
                        requires: &signature.requires,
                        ensures: &signature.ensures,
                    };
                    self.call(&contract, &values, arg_spans, *span, &x, ctx);
                }
            }
            Rhs::Construct {
                ctor,
                args,
                arg_spans,
            } => {
                self.fields(ctor, args, arg_spans, ctx);
                if let Some(tag) = self.tag(&x) {
                    ctx.define(F::eq(tag, F::Int(ctor.index.to_string())));
                }
                for (index, atom) in args.iter().enumerate() {
                    let value = self.atom(atom, ctx);
                    if let Some(field) = self.field(&x, ctor.index, index)
                        && field.f.sort() == value.f.sort()
                    {
                        ctx.define(F::eq(field.f, value.f));
                    }
                }
            }
            Rhs::Field { value, index, .. } => {
                let record = self.atom(value, ctx);
                if let Some(projection) = self.field(&record, 0, *index)
                    && projection.f.sort() == x.f.sort()
                {
                    ctx.define(F::eq(x.f.clone(), projection.f.clone()));
                    if name.starts_with('%') {
                        self.defs.insert(name.to_owned(), projection.f);
                    }
                }
            }
            Rhs::List(items) => {
                if let Some(len) = self.len(&x) {
                    ctx.define(F::eq(len, F::Int(items.len().to_string())));
                }
            }
            Rhs::Lambda { params, body } => {
                let mut inner = ctx.clone();
                for (param, ty) in params {
                    self.bind(&mut inner, param, ty);
                }
                let mut leaves = Vec::new();
                self.walk(body, &inner, &mut leaves);
            }
            Rhs::Checked {
                value,
                formula,
                ctor,
                args,
                arg_spans,
                ..
            } => {
                self.fields(ctor, args, arg_spans, ctx);
                let checked = self.atom(value, ctx);
                let env = Env {
                    vars: &|name| Some(own(ctx, name)),
                    it: Some(checked.clone()),
                    result: None,
                };
                if let Some(fact) = self.term(formula, &env) {
                    ctx.assume(fact.f);
                }
                if checked.f.sort() == x.f.sort() {
                    ctx.define(F::eq(x.f.clone(), checked.f));
                }
            }
            Rhs::Declassify {
                policy,
                arg,
                arg_span,
                ..
            } => {
                let key = (policy.module.as_str(), policy.name.as_str());
                if let Some((from, to)) = self.index.policies.get(&key) {
                    let value = self.atom(arg, ctx);
                    self.refinement_sites(from, &value, *arg_span, ctx);
                    let result = Tv {
                        f: x.f.clone(),
                        ty: (*to).clone(),
                    };
                    self.assume_refinements(ctx, &result);
                }
            }
        }
    }

    /// A call of a function or an operation: refinements of its arguments, its `requires`,
    /// then the refinements of its result and its `ensures` as facts about `x` (T-Call, T-Op).
    fn call(
        &mut self,
        contract: &Contract<'_>,
        values: &[Tv],
        arg_spans: &[Span],
        span: Span,
        x: &Tv,
        ctx: &mut Ctx,
    ) {
        for (((_, param_ty), value), arg_span) in contract.params.iter().zip(values).zip(arg_spans)
        {
            self.refinement_sites(param_ty, value, *arg_span, ctx);
        }
        let by_name: BTreeMap<&str, Tv> = contract
            .params
            .iter()
            .zip(values)
            .map(|((name, _), value)| (name.as_str(), value.clone()))
            .collect();
        let texts: BTreeMap<&str, String> = contract
            .params
            .iter()
            .zip(arg_spans)
            .map(|((name, _), span)| (name.as_str(), self.text(*span)))
            .collect();
        let vars = |name: &str| by_name.get(name).cloned();
        for clause in contract.requires {
            let env = Env {
                vars: &vars,
                it: None,
                result: None,
            };
            let goal = self.term(&clause.term, &env);
            let text = print_term_with(&clause.term, &|term| match term {
                Term::Var(name) => texts.get(name.as_str()).cloned(),
                _ => None,
            });
            self.site(
                ObligationKind::Precondition,
                ctx,
                goal.map(|g| g.f),
                text,
                span,
                clause.span.map(|span| (span, "requires clause")),
            );
        }
        let result = Tv {
            f: x.f.clone(),
            ty: contract.result.clone(),
        };
        self.assume_refinements(ctx, &result);
        for clause in contract.ensures {
            let env = Env {
                vars: &vars,
                it: None,
                result: Some(x.clone()),
            };
            if let Some(fact) = self.term(&clause.term, &env) {
                ctx.assume(fact.f);
            }
        }
    }

    /// The fields of a constructed value or a failure: refinement obligations at each.
    fn fields(&mut self, ctor: &Ctor, args: &[Atom], arg_spans: &[Span], ctx: &Ctx) {
        let variants = if self.index.prelude_kind(&ctor.kind.module, &ctor.kind.name) {
            None
        } else {
            self.index
                .kinds
                .get(&(ctor.kind.module.as_str(), ctor.kind.name.as_str()))
        };
        let Some(kind) = variants else {
            return;
        };
        let Some(variant) = kind.variants.get(ctor.index) else {
            return;
        };
        let params: Vec<(Name, Type)> = variant.fields.clone();
        for (((_, field_ty), atom), span) in params.iter().zip(args).zip(arg_spans) {
            let value = self.atom(atom, ctx);
            self.refinement_sites(field_ty, &value, *span, ctx);
        }
    }

    /// Arguments into declared parameters: refinement obligations.
    fn arguments(&mut self, params: &[(Name, Type)], args: &[Atom], arg_spans: &[Span], ctx: &Ctx) {
        for (((_, ty), atom), span) in params.iter().zip(args).zip(arg_spans) {
            let value = self.atom(atom, ctx);
            self.refinement_sites(ty, &value, *span, ctx);
        }
    }

    /// A value flowing into `target`: one `refinement` obligation per refinement, at the
    /// value (`spec/12 §3`).
    fn refinement_sites(&mut self, target: &Type, value: &Tv, span: Span, ctx: &Ctx) {
        let value_text = self.text(span);
        for refinement in refinements(target) {
            let env = Env {
                vars: &|_| None,
                it: Some(value.clone()),
                result: None,
            };
            let goal = self.term(&refinement.clause.term, &env);
            let text = instantiate(&refinement.clause.term, &value_text);
            self.site(
                ObligationKind::Refinement,
                ctx,
                goal.map(|g| g.f),
                text,
                span,
                refinement
                    .clause
                    .span
                    .map(|span| (span, clause_role(refinement))),
            );
        }
    }

    // ----- values -----

    fn atom(&mut self, atom: &Atom, ctx: &Ctx) -> Tv {
        match atom {
            Atom::Var(name) => own(ctx, name),
            Atom::Int(digits) => Tv {
                f: F::Int(digits.clone()),
                ty: base_type(Base::Int),
            },
            Atom::Bool(value) => Tv {
                f: F::Bool(*value),
                ty: base_type(Base::Bool),
            },
            Atom::Text(text) => text_literal(text),
            Atom::Unit => Tv {
                f: F::Const(Const {
                    name: "unit".to_owned(),
                    sort: UNIT.to_owned(),
                    role: ConstRole::Internal,
                }),
                ty: base_type(Base::Unit),
            },
        }
    }

    /// The tag of a value of a kind, `i!tag!<Sort>`, when its kind is a sum.
    fn tag(&self, value: &Tv) -> Option<F> {
        let Type::Base {
            base: Base::Named { module, name, .. },
            ..
        } = &value.ty
        else {
            return None;
        };
        let names = self.index.variant_names(module, name);
        let record = self
            .index
            .kinds
            .get(&(module.as_str(), name.as_str()))
            .is_some_and(|kind| kind.record);
        if names.is_empty() || record {
            return None;
        }
        Some(tag_of(&value.f, module, name, names.len()))
    }

    /// Field `index` of `variant` of a value: `u!f!<field>!<Sort>` for a record,
    /// `u!f!<field>!<Variant>!<Sort>` for a variant of a sum, so that variants sharing a field
    /// name with different types have distinct functions, and `i!f!<field>!<Sort>` for the
    /// fields of the prelude's kinds (ADR 0038 D8).
    fn field(&self, value: &Tv, variant: usize, index: usize) -> Option<Tv> {
        let Type::Base {
            base: Base::Named { module, name, .. },
            ..
        } = &value.ty
        else {
            return None;
        };
        let field_ty = self.index.field_type(&value.ty, variant, index)?;
        let sort = value.f.sort();
        let symbol = if self.index.prelude_kind(module, name) {
            let field = prelude_variants(name)
                .and_then(|variants| variants.get(variant))
                .and_then(|(_, fields)| fields.get(index))?;
            format!("i!f!{field}!{sort}")
        } else {
            let kind = self.index.kinds.get(&(module.as_str(), name.as_str()))?;
            let declared = kind.variants.get(variant)?;
            let (field, _) = declared.fields.get(index)?;
            if kind.record {
                format!("u!f!{field}!{sort}")
            } else {
                format!("u!f!{field}!{}!{sort}", declared.name)
            }
        };
        let variants = self.index.variant_names(module, name).len();
        Some(Tv {
            f: F::App {
                fun: SmtFun {
                    name: symbol,
                    args: vec![sort],
                    result: self::sort(&field_ty),
                    role: FunRole::Field {
                        module: module.clone(),
                        kind: name.clone(),
                        variant,
                        index,
                        variants,
                    },
                },
                args: vec![value.f.clone()],
            },
            ty: field_ty,
        })
    }

    /// `.len` of a text or list, `i!len!<Sort>`.
    fn len(&self, value: &Tv) -> Option<F> {
        let sort = value.f.sort();
        let is_text_or_list = sort == TEXT || sort.starts_with(LIST);
        is_text_or_list.then(|| len_of(&value.f))
    }

    /// A primitive of code as a term, when it is one of the refinement language (`spec/10 §3`:
    /// "RL-expressible results are selfified").
    fn prim(&self, prim: Prim, values: &[Tv]) -> Option<F> {
        let arg = |i: usize| values.get(i).cloned();
        Some(match prim {
            Prim::Add | Prim::Sub => {
                let (a, b) = (arg(0)?.f, arg(1)?.f);
                F::op(if prim == Prim::Add { "+" } else { "-" }, vec![a, b])
            }
            Prim::Mul => arithmetic(BinOp::Mul, arg(0)?.f, arg(1)?.f),
            Prim::Div => arithmetic(BinOp::Div, arg(0)?.f, arg(1)?.f),
            Prim::Mod => arithmetic(BinOp::Mod, arg(0)?.f, arg(1)?.f),
            Prim::Neg => F::op("-", vec![arg(0)?.f]),
            Prim::Not => F::not(arg(0)?.f),
            Prim::Eq | Prim::Ne => {
                let (a, b) = (arg(0)?, arg(1)?);
                let eq = self.equal(&a, &b)?;
                if prim == Prim::Eq { eq } else { F::not(eq) }
            }
            Prim::Lt | Prim::Le | Prim::Gt | Prim::Ge => {
                let (a, b) = (arg(0)?, arg(1)?);
                let op = match prim {
                    Prim::Lt => BinOp::Lt,
                    Prim::Le => BinOp::Le,
                    Prim::Gt => BinOp::Gt,
                    _ => BinOp::Ge,
                };
                self.order(op, &a, &b)?
            }
            Prim::Len => self.len(&arg(0)?)?,
            Prim::Concat | Prim::ToText => return None,
        })
    }

    /// Equality of two values: of tags for a sum without payloads, of the values otherwise.
    fn equal(&self, a: &Tv, b: &Tv) -> Option<F> {
        if a.f.sort() != b.f.sort() {
            return None;
        }
        if let Type::Base {
            base: Base::Named { module, name, .. },
            ..
        } = &a.ty
            && self.index.payload_free(module, name)
        {
            return Some(F::eq(self.tag(a)?, self.tag(b)?));
        }
        Some(F::eq(a.f.clone(), b.f.clone()))
    }

    /// Order of two values: `Int` natively, a sum without payloads by its tags, `Text` by an
    /// uninterpreted relation (DECISION D-115).
    fn order(&self, op: BinOp, a: &Tv, b: &Tv) -> Option<F> {
        let symbol = match op {
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            _ => ">=",
        };
        let sort = a.f.sort();
        if sort == "Int" {
            return Some(F::op(symbol, vec![a.f.clone(), b.f.clone()]));
        }
        if sort == TEXT {
            let name = match op {
                BinOp::Lt => "i!lt!Text",
                BinOp::Le => "i!le!Text",
                BinOp::Gt => "i!gt!Text",
                _ => "i!ge!Text",
            };
            return Some(F::App {
                fun: SmtFun {
                    name: name.to_owned(),
                    args: vec![TEXT.to_owned(), TEXT.to_owned()],
                    result: "Bool".to_owned(),
                    role: FunRole::Other,
                },
                args: vec![a.f.clone(), b.f.clone()],
            });
        }
        Some(F::op(symbol, vec![self.tag(a)?, self.tag(b)?]))
    }

    /// A call of a predicate as a term; a reflect predicate's definition is kept for the
    /// queries that use it.
    fn predicate(
        &mut self,
        path: &una_core::Path,
        params: &[(Name, Type)],
        formula: Option<&Term>,
        args: &[Tv],
    ) -> Option<F> {
        let name = format!("u!p!{}", path.qualified());
        let sorts: Vec<Sort> = args.iter().map(|a| a.f.sort()).collect();
        if let Some(formula) = formula
            && !self.definitions.contains_key(&name)
        {
            let locals: Vec<Tv> = params
                .iter()
                .zip(&sorts)
                .map(|((param, ty), sort)| Tv {
                    f: F::Local {
                        name: param.clone(),
                        sort: sort.clone(),
                    },
                    ty: ty.clone(),
                })
                .collect();
            let by_name: BTreeMap<&str, Tv> = params
                .iter()
                .zip(&locals)
                .map(|((param, _), local)| (param.as_str(), local.clone()))
                .collect();
            let vars = |n: &str| by_name.get(n).cloned();
            let env = Env {
                vars: &vars,
                it: None,
                result: None,
            };
            let body = self.term(formula, &env)?;
            self.definitions.insert(
                name.clone(),
                Definition {
                    params: params
                        .iter()
                        .zip(&sorts)
                        .map(|((param, _), sort)| (param.clone(), sort.clone()))
                        .collect(),
                    body: body.f,
                },
            );
        }
        Some(F::App {
            fun: SmtFun {
                name,
                args: sorts,
                result: "Bool".to_owned(),
                role: FunRole::Predicate,
            },
            args: args.iter().map(|a| a.f.clone()).collect(),
        })
    }

    /// A term of the refinement language as a formula; `None` when it uses a value the
    /// encoding does not have.
    fn term(&mut self, term: &Term, env: &Env<'_>) -> Option<Tv> {
        let int = || base_type(Base::Int);
        let boolean = || base_type(Base::Bool);
        Some(match term {
            Term::Var(name) => (env.vars)(name)?,
            Term::It => env.it.clone()?,
            Term::Result => env.result.clone()?,
            Term::Int(digits) => Tv {
                f: F::Int(digits.clone()),
                ty: int(),
            },
            Term::Bool(value) => Tv {
                f: F::Bool(*value),
                ty: boolean(),
            },
            Term::Text(text) => text_literal(text),
            Term::Unit => return None,
            Term::Ctor(_) => return None,
            Term::Field { value, index, .. } => {
                let value = self.term(value, env)?;
                self.field(&value, 0, *index)?
            }
            Term::Len(value) => {
                let value = self.term(value, env)?;
                Tv {
                    f: self.len(&value)?,
                    ty: int(),
                }
            }
            Term::Neg(value) => Tv {
                f: F::op("-", vec![self.term(value, env)?.f]),
                ty: int(),
            },
            Term::Not(value) => Tv {
                f: F::not(self.term(value, env)?.f),
                ty: boolean(),
            },
            Term::Is { value, ctor } => {
                let value = self.term(value, env)?;
                Tv {
                    f: F::eq(self.tag(&value)?, F::Int(ctor.index.to_string())),
                    ty: boolean(),
                }
            }
            Term::In {
                value,
                low,
                high,
                exclusive,
            } => {
                let value = self.term(value, env)?.f;
                let upper = if *exclusive { "<" } else { "<=" };
                Tv {
                    f: F::and(vec![
                        F::op("<=", vec![F::Int(low.clone()), value.clone()]),
                        F::op(upper, vec![value, F::Int(high.clone())]),
                    ]),
                    ty: boolean(),
                }
            }
            Term::Call { predicate, args } => {
                let values: Vec<Tv> = args
                    .iter()
                    .map(|a| self.term(a, env))
                    .collect::<Option<_>>()?;
                let key = (predicate.module.as_str(), predicate.name.as_str());
                let (params, formula) = self.index.predicates.get(&key).copied()?;
                Tv {
                    f: self.predicate(predicate, params, formula, &values)?,
                    ty: boolean(),
                }
            }
            Term::Binary { op, left, right } => self.binary(*op, left, right, env)?,
        })
    }

    fn binary(&mut self, op: BinOp, left: &Term, right: &Term, env: &Env<'_>) -> Option<Tv> {
        let boolean = base_type(Base::Bool);
        // A variant without fields compares by its tag.
        if let (Term::Ctor(ctor), other) | (other, Term::Ctor(ctor)) = (left, right)
            && matches!(op, BinOp::Eq | BinOp::Ne)
        {
            let value = self.term(other, env)?;
            let eq = F::eq(self.tag(&value)?, F::Int(ctor.index.to_string()));
            let f = if op == BinOp::Eq { eq } else { F::not(eq) };
            return Some(Tv { f, ty: boolean });
        }
        if let (Term::Ctor(ctor), _) | (_, Term::Ctor(ctor)) = (left, right)
            && matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
        {
            let k = F::Int(ctor.index.to_string());
            let ctor_on_left = matches!(left, Term::Ctor(_));
            let other = if ctor_on_left { right } else { left };
            let value = self.term(other, env)?;
            let tag = self.tag(&value)?;
            let (l, r) = if ctor_on_left { (k, tag) } else { (tag, k) };
            let symbol = match op {
                BinOp::Lt => "<",
                BinOp::Le => "<=",
                BinOp::Gt => ">",
                _ => ">=",
            };
            return Some(Tv {
                f: F::op(symbol, vec![l, r]),
                ty: boolean,
            });
        }
        let l = self.term(left, env)?;
        let r = self.term(right, env)?;
        Some(match op {
            BinOp::And | BinOp::Or | BinOp::Implies => Tv {
                f: F::op(
                    match op {
                        BinOp::And => "and",
                        BinOp::Or => "or",
                        _ => "=>",
                    },
                    vec![l.f, r.f],
                ),
                ty: boolean,
            },
            BinOp::Eq => Tv {
                f: self.equal(&l, &r)?,
                ty: boolean,
            },
            BinOp::Ne => Tv {
                f: F::not(self.equal(&l, &r)?),
                ty: boolean,
            },
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => Tv {
                f: self.order(op, &l, &r)?,
                ty: boolean,
            },
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => Tv {
                f: arithmetic(op, l.f, r.f),
                ty: base_type(Base::Int),
            },
        })
    }

    // ----- obligations -----

    /// An obligation at one site: `Π ⊢ goal`.
    fn site(
        &mut self,
        kind: ObligationKind,
        ctx: &Ctx,
        goal: Option<F>,
        text: String,
        span: Span,
        related: Option<(Span, &'static str)>,
    ) {
        let mut assertions: Vec<F> = ctx.hyps.iter().map(|h| h.f.clone()).collect();
        // A goal outside the encoding is not assumed proven: its query has no negated goal
        // and stays satisfiable, so the obligation is refuted, never silently proven.
        assertions.push(F::not(goal.unwrap_or(F::Bool(false))));
        let hypotheses = self.shown_hypotheses(&ctx.hyps);
        self.push(
            kind,
            text,
            span,
            related,
            Vec::new(),
            hypotheses,
            assertions,
            ctx,
        );
    }

    /// An obligation over every normal return path: `Π ∧ (path₁ ∨ … ∨ pathₙ) ⊢ goal`, where
    /// each path fixes `result` (DECISION D-113).
    #[allow(clippy::too_many_arguments)]
    fn returns(
        &mut self,
        kind: ObligationKind,
        entry: &Ctx,
        leaves: &[Leaf],
        result: &Tv,
        goal: Option<Tv>,
        text: String,
        source: Span,
        related: Option<(Span, &'static str)>,
    ) {
        if leaves.is_empty() {
            return;
        }
        let base = entry.hyps.len();
        let mut assertions: Vec<F> = entry.hyps.iter().map(|h| h.f.clone()).collect();
        let mut selectors = Vec::new();
        for (number, leaf) in leaves.iter().enumerate() {
            let selector = path_selector(number + 1);
            assertions.push(F::eq(selector.clone(), self.path(leaf, base, result)));
            selectors.push(selector);
        }
        assertions.push(F::or(selectors));
        assertions.push(F::not(goal.map_or(F::Bool(false), |g| g.f)));
        let hypotheses = self.shown_hypotheses(&entry.hyps);
        let paths = leaves.iter().map(|leaf| leaf.span).collect();
        let mut ctx = entry.clone();
        for leaf in leaves {
            for hyp in leaf.hyps.iter().skip(base) {
                ctx.hyps.push(hyp.clone());
            }
        }
        ctx.types.insert("result".to_owned(), result.ty.clone());
        self.push(
            kind, text, source, related, paths, hypotheses, assertions, &ctx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        kind: ObligationKind,
        goal: String,
        source: Span,
        related: Option<(Span, &'static str)>,
        paths: Vec<Span>,
        hypotheses: Vec<String>,
        assertions: Vec<F>,
        ctx: &Ctx,
    ) {
        let mut query = Query {
            assertions,
            definitions: BTreeMap::new(),
        };
        let (facts, listed) = self.ground_facts(&query);
        if !facts.is_empty() {
            let mut all = facts;
            all.append(&mut query.assertions);
            query.assertions = all;
        }
        // The refinements of record fields are listed first: `spec/13 §2` shows them.
        let mut hypotheses = hypotheses;
        let mut shown_facts: Vec<String> = Vec::new();
        for fact in &listed {
            for part in conjuncts(fact) {
                let text = self.show_formula(part);
                if !shown_facts.contains(&text) && !hypotheses.contains(&text) {
                    shown_facts.push(text);
                }
            }
        }
        shown_facts.append(&mut hypotheses);
        let hypotheses = shown_facts;
        self.add_definitions(&mut query);
        let shown = self.shown(&query, ctx);
        let normalized: String = goal.split_whitespace().collect::<Vec<_>>().join(" ");
        let id = obligation_id(self.module, &self.declaration, kind, &normalized);
        self.obligations.push(Obligation {
            id,
            kind,
            module: self.module.to_owned(),
            declaration: self.declaration.clone(),
            goal: normalized,
            source,
            related,
            paths,
            hypotheses,
            query,
            shown,
        });
    }

    /// The facts about the terms of a query that no quantifier states (`spec/12 §2, §4`,
    /// DECISION D-114): lengths are at least 0, tags are in range, and fields satisfy their
    /// declared refinements.
    fn ground_facts(&mut self, query: &Query) -> (Vec<F>, Vec<F>) {
        let mut facts: Vec<F> = Vec::new();
        let mut listed: Vec<F> = Vec::new();
        let mut seen: BTreeSet<F> = BTreeSet::new();
        let mut pending: Vec<F> = query.applications().into_iter().collect();
        let mut rounds = 0;
        while !pending.is_empty() && rounds < 6 {
            rounds += 1;
            let mut next = Vec::new();
            for term in pending {
                if !seen.insert(term.clone()) {
                    continue;
                }
                let mut new = Vec::new();
                match &term {
                    F::App { fun, args } => match &fun.role {
                        FunRole::Len => {
                            new.push(F::op(">=", vec![term.clone(), F::Int("0".to_owned())]));
                        }
                        FunRole::Tag { variants, .. } => new.push(F::and(vec![
                            F::op("<=", vec![F::Int("0".to_owned()), term.clone()]),
                            F::op("<", vec![term.clone(), F::Int(variants.to_string())]),
                        ])),
                        FunRole::Field {
                            module,
                            kind,
                            variant,
                            index,
                            variants,
                        } => {
                            let declared = self
                                .index
                                .kinds
                                .get(&(module.as_str(), kind.as_str()))
                                .and_then(|k| k.variants.get(*variant))
                                .and_then(|v| v.fields.get(*index))
                                .map(|(_, ty)| ty.clone());
                            if let Some(declared) = declared {
                                let value = Tv {
                                    f: term.clone(),
                                    ty: declared.clone(),
                                };
                                let owner = args.first().cloned();
                                for refinement in refinements(&declared) {
                                    let env = Env {
                                        vars: &|_| None,
                                        it: Some(value.clone()),
                                        result: None,
                                    };
                                    let Some(fact) = self.term(&refinement.clause.term, &env)
                                    else {
                                        continue;
                                    };
                                    if *variants <= 1 {
                                        listed.push(fact.f.clone());
                                    }
                                    let fact = match (&owner, *variants > 1) {
                                        (Some(owner), true) => {
                                            let tag = tag_of(owner, module, kind, *variants);
                                            F::op(
                                                "=>",
                                                vec![
                                                    F::eq(tag, F::Int(variant.to_string())),
                                                    fact.f,
                                                ],
                                            )
                                        }
                                        _ => fact.f,
                                    };
                                    new.push(fact);
                                }
                            }
                        }
                        FunRole::Predicate | FunRole::Other => {}
                    },
                    F::Const(Const {
                        role: ConstRole::Text(text),
                        ..
                    }) => {
                        new.push(F::eq(
                            len_of(&term),
                            F::Int(text.chars().count().to_string()),
                        ));
                    }
                    _ => {}
                }
                for fact in new {
                    let probe = Query {
                        assertions: vec![fact.clone()],
                        definitions: BTreeMap::new(),
                    };
                    next.extend(probe.applications());
                    facts.push(fact);
                }
            }
            pending = next;
        }
        (facts, listed)
    }

    /// Adds the definitions of the reflect predicates the query uses, and of those they use.
    fn add_definitions(&self, query: &mut Query) {
        let mut pending: Vec<String> = Vec::new();
        let collect = |f: &F, out: &mut Vec<String>| {
            let probe = Query {
                assertions: vec![f.clone()],
                definitions: BTreeMap::new(),
            };
            for term in probe.applications() {
                if let F::App { fun, .. } = term
                    && fun.role == FunRole::Predicate
                {
                    out.push(fun.name.clone());
                }
            }
        };
        for assertion in &query.assertions {
            collect(assertion, &mut pending);
        }
        while let Some(name) = pending.pop() {
            if query.definitions.contains_key(&name) {
                continue;
            }
            if let Some(definition) = self.definitions.get(&name) {
                collect(&definition.body, &mut pending);
                query.definitions.insert(name, definition.clone());
            }
        }
    }

    /// The terms a counterexample shows: named binders, `result`, and the fields and lengths
    /// over them, by name (DECISION D-119).
    fn shown(&self, query: &Query, ctx: &Ctx) -> Vec<Shown> {
        let mut out: BTreeMap<String, Shown> = BTreeMap::new();
        for term in query.applications() {
            match &term {
                F::Const(Const {
                    name,
                    role: ConstRole::Plain,
                    ..
                }) if !name.starts_with('%') => {
                    let Some(ty) = ctx.types.get(name) else {
                        continue;
                    };
                    let source = source_name(name);
                    let value = Tv {
                        f: term.clone(),
                        ty: ty.clone(),
                    };
                    out.insert(
                        source.clone(),
                        Shown {
                            name: source,
                            term: term.clone(),
                            show: self.show_kind(&value),
                        },
                    );
                }
                F::App { fun, args }
                    if matches!(fun.role, FunRole::Field { .. } | FunRole::Len) =>
                {
                    let Some(owner) = args.first().and_then(|a| self.term_name(a)) else {
                        continue;
                    };
                    let (name, ty) = match &fun.role {
                        FunRole::Field {
                            module,
                            kind,
                            variant,
                            index,
                            ..
                        } => {
                            let field = source_symbol(fun);
                            let ty = self
                                .index
                                .kinds
                                .get(&(module.as_str(), kind.as_str()))
                                .and_then(|k| k.variants.get(*variant))
                                .and_then(|v| v.fields.get(*index))
                                .map_or_else(|| base_type(Base::Unknown), |(_, t)| t.clone());
                            (format!("{owner}.{field}"), ty)
                        }
                        _ => (format!("{owner}.len"), base_type(Base::Int)),
                    };
                    let value = Tv {
                        f: term.clone(),
                        ty,
                    };
                    out.insert(
                        name.clone(),
                        Shown {
                            name,
                            term: term.clone(),
                            show: self.show_kind(&value),
                        },
                    );
                }
                _ => {}
            }
        }
        out.into_values().collect()
    }

    /// The source name of a shown term: a named binder or a field of one.
    fn term_name(&self, f: &F) -> Option<String> {
        match f {
            F::Const(Const {
                name,
                role: ConstRole::Plain,
                ..
            }) if !name.starts_with('%') => Some(source_name(name)),
            F::App { fun, args } if matches!(fun.role, FunRole::Field { .. }) => {
                let owner = self.term_name(args.first()?)?;
                Some(format!("{owner}.{}", source_symbol(fun)))
            }
            _ => None,
        }
    }

    fn show_kind(&self, value: &Tv) -> Show {
        let sort = value.f.sort();
        if sort == "Int" || sort == "Bool" {
            return Show::Value;
        }
        if let Type::Base {
            base: Base::Named { module, name, .. },
            ..
        } = &value.ty
        {
            if let Some(tag) = self.tag(value) {
                return Show::Variant {
                    tag,
                    names: self.index.variant_names(module, name),
                };
            }
            return Show::Opaque(name.clone());
        }
        Show::Opaque(type_name(&value.ty))
    }

    /// How a binder shows: a temporary through its definition or its source text.
    fn show_name(&self, name: &str) -> String {
        self.show_binder(name).0
    }

    fn show_binder(&self, name: &str) -> (String, u8) {
        if let Some(def) = self.defs.get(name) {
            return self.show(def);
        }
        match self.temps.get(name) {
            Some(text) => (text.clone(), text_level(text)),
            None => (source_name(name), 1),
        }
    }

    fn show_temp(&mut self, name: &str, atom: &Atom) {
        if name.starts_with('%')
            && let Atom::Var(other) = atom
        {
            let shown = self.show_name(other);
            self.temps.insert(name.to_owned(), shown);
        }
    }

    /// The listed hypotheses: those that are not definitions of temporaries, as text.
    fn shown_hypotheses(&self, hyps: &[Hyp]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for hyp in hyps.iter().filter(|h| h.shown) {
            for part in conjuncts(&hyp.f) {
                let text = self.show_formula(part);
                if !out.contains(&text) {
                    out.push(text);
                }
            }
        }
        out
    }

    /// A formula in a readable form of the surface syntax.
    fn show_formula(&self, f: &F) -> String {
        self.show(f).0
    }

    /// A formula as surface syntax, and its level in the precedence table of `spec/01 §7`:
    /// 1 binds tightest, 9 is `implies` and 10 an `if`. An operand shows in parentheses when
    /// its level is above the one its position allows.
    fn show(&self, f: &F) -> (String, u8) {
        let operand = |f: &F, max: u8| self.operand(f, max);
        let binary = |a: &F, op: &str, b: &F, level: u8| self.show_binary(a, op, b, level);
        match f {
            F::Int(digits) => (digits.clone(), if digits.starts_with('-') { 2 } else { 1 }),
            F::Bool(value) => (value.to_string(), 1),
            F::Const(Const { name, role, .. }) => match role {
                ConstRole::Text(text) => (una_core::quote(text), 1),
                _ => self.show_binder(name),
            },
            F::Local { name, .. } => (source_name(name), 1),
            F::App { fun, args } => {
                let first = || args.first().map_or_else(String::new, |a| operand(a, 1));
                let base = source_symbol(fun);
                match &fun.role {
                    FunRole::Field { .. } => (format!("{}.{base}", first()), 1),
                    FunRole::Len => (format!("{}.len", first()), 1),
                    FunRole::Tag { .. } => (format!("tag({})", first()), 1),
                    FunRole::Predicate => {
                        let shown: Vec<String> = args.iter().map(|a| operand(a, 10)).collect();
                        let short = base.rsplit('.').next().unwrap_or(base);
                        (format!("{short}({})", shown.join(", ")), 1)
                    }
                    FunRole::Other => {
                        let (op, level) = match base {
                            "mul" => ("*", 3),
                            "div" => ("/", 3),
                            "mod" => ("mod", 3),
                            "lt" => ("<", 5),
                            "le" => ("<=", 5),
                            "gt" => (">", 5),
                            _ => (">=", 5),
                        };
                        match args.as_slice() {
                            [a, b] => binary(a, op, b, level),
                            _ => (format!("{base}(..)"), 1),
                        }
                    }
                }
            }
            F::Op { op, args } => match (*op, self.resolved(args).as_slice()) {
                ("not", [F::Op { op: "=", args }]) if args.len() == 2 => {
                    match self.resolved(args).as_slice() {
                        [a, b] => self.show_equal(a, b, true),
                        _ => (String::new(), 1),
                    }
                }
                ("not", [a]) => (format!("not {}", operand(a, 6)), 6),
                ("-", [a]) => (format!("-{}", operand(a, 1)), 2),
                ("=", [a, b]) => self.show_equal(a, b, false),
                ("=>", [a, b]) => (format!("{} implies {}", operand(a, 8), operand(b, 9)), 9),
                ("ite", [c, a, F::Bool(false)]) | ("and", [c, a])
                    if let Some(range) = self.show_range(c, a) =>
                {
                    range
                }
                ("ite", [c, a, b]) => match (a, b) {
                    (F::Bool(true), F::Bool(false)) => self.show(c),
                    (F::Bool(false), F::Bool(true)) => (format!("not {}", operand(c, 6)), 6),
                    (_, F::Bool(false)) => (format!("{} and {}", operand(c, 7), operand(a, 7)), 7),
                    (F::Bool(true), _) => (format!("{} or {}", operand(c, 8), operand(b, 8)), 8),
                    (_, F::Bool(true)) => {
                        (format!("{} implies {}", operand(c, 8), operand(a, 9)), 9)
                    }
                    _ => (
                        format!(
                            "if {} then {} else {}",
                            operand(c, 10),
                            operand(a, 9),
                            operand(b, 10)
                        ),
                        10,
                    ),
                },
                ("and" | "or", parts) => {
                    let level = if *op == "and" { 7 } else { 8 };
                    let shown: Vec<String> = parts.iter().map(|p| operand(p, level)).collect();
                    (shown.join(&format!(" {op} ")), level)
                }
                ("+" | "-" | "*" | "div" | "mod" | "<" | "<=" | ">" | ">=", [a, b]) => {
                    let (op, level) = match *op {
                        "+" => ("+", 4),
                        "-" => ("-", 4),
                        "*" => ("*", 3),
                        "div" => ("/", 3),
                        "mod" => ("mod", 3),
                        other => (other, 5),
                    };
                    binary(a, op, b, level)
                }
                (op, parts) => {
                    let shown: Vec<String> = parts.iter().map(|p| operand(p, 4)).collect();
                    (shown.join(&format!(" {op} ")), 5)
                }
            },
        }
    }

    /// The formulas, each temporary replaced by its definition.
    fn resolved<'f>(&'f self, fs: &'f [F]) -> Vec<&'f F> {
        fs.iter().map(|f| self.resolve(f)).collect()
    }

    /// A temporary's definition, followed to a formula that is not one.
    fn resolve<'f>(&'f self, f: &'f F) -> &'f F {
        let mut f = f;
        while let F::Const(Const { name, .. }) = f
            && let Some(def) = self.defs.get(name)
        {
            f = def;
        }
        f
    }

    /// `lo <= x and x <= hi` as `x in lo..hi`, and `x in lo..<hi` with `<`.
    fn show_range(&self, low: &F, high: &F) -> Option<(String, u8)> {
        let (
            F::Op {
                op: "<=",
                args: lower,
            },
            F::Op {
                op: upper,
                args: higher,
            },
        ) = (self.resolve(low), self.resolve(high))
        else {
            return None;
        };
        let (lower, higher) = (self.resolved(lower), self.resolved(higher));
        let ([F::Int(lo), x], [y, F::Int(hi)]) = (lower.as_slice(), higher.as_slice()) else {
            return None;
        };
        let range = match *upper {
            "<=" => "..",
            "<" => "..<",
            _ => return None,
        };
        if x != y {
            return None;
        }
        let shown = self.operand(x, 4);
        Some((format!("{shown} in {lo}{range}{hi}"), 5))
    }

    /// `a == b` or `a != b`; the variant of a value against a literal shows as `is`.
    fn show_equal(&self, a: &F, b: &F, negated: bool) -> (String, u8) {
        if let (
            F::App {
                fun:
                    SmtFun {
                        role: FunRole::Tag { module, kind, .. },
                        ..
                    },
                args,
            },
            F::Int(digits),
        ) = (a, b)
            && let [value] = args.as_slice()
            && let Ok(index) = digits.parse::<usize>()
            && let Some(variant) = self.index.variant_names(module, kind).get(index).cloned()
        {
            let is = format!("{} is {variant}", self.operand(value, 4));
            return if negated {
                (format!("not {is}"), 6)
            } else {
                (is, 5)
            };
        }
        self.show_binary(a, if negated { "!=" } else { "==" }, b, 5)
    }

    /// An operand at a position that allows levels up to `max`, in parentheses above it.
    fn operand(&self, f: &F, max: u8) -> String {
        let (text, level) = self.show(f);
        if level > max {
            format!("({text})")
        } else {
            text
        }
    }

    /// `a op b` at `level`: `*`, `/`, `mod`, `+` and `-` associate to the left, and
    /// comparisons do not chain.
    fn show_binary(&self, a: &F, op: &str, b: &F, level: u8) -> (String, u8) {
        let (left, right) = if level == 5 {
            (4, 4)
        } else {
            (level, level - 1)
        };
        (
            format!("{} {op} {}", self.operand(a, left), self.operand(b, right)),
            level,
        )
    }
}

/// The precedence level of a temporary's source text: 1 when it reads as one operand, with
/// no space outside brackets and text literals; 2 for a negation; 10 otherwise.
fn text_level(text: &str) -> u8 {
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for c in text.chars() {
        if quoted {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                quoted = false;
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && depth == 0 => return 10,
            _ => {}
        }
    }
    if text.starts_with('-') { 2 } else { 1 }
}

/// The value of a binder in scope.
fn own(ctx: &Ctx, name: &str) -> Tv {
    let ty = ctx
        .types
        .get(name)
        .cloned()
        .unwrap_or_else(|| base_type(Base::Unknown));
    Tv {
        f: constant(name, &ty),
        ty,
    }
}

fn constant(name: &str, ty: &Type) -> F {
    F::Const(Const {
        name: name.to_owned(),
        sort: sort(ty),
        role: ConstRole::Plain,
    })
}

/// A text literal: one constant per distinct text, `i!lit!<hex>` by its bytes (`spec/12 §4`).
fn text_literal(text: &str) -> Tv {
    let hex: String = text.bytes().map(|b| format!("{b:02x}")).collect();
    Tv {
        f: F::Const(Const {
            name: hex,
            sort: TEXT.to_owned(),
            role: ConstRole::Text(text.to_owned()),
        }),
        ty: base_type(Base::Text),
    }
}

/// The tag of `value`, of the sum at (`module`, `kind`) with `variants` variants,
/// `i!tag!<Sort>`.
fn tag_of(value: &F, module: &str, kind: &str, variants: usize) -> F {
    let sort = value.sort();
    F::App {
        fun: SmtFun {
            name: format!("i!tag!{sort}"),
            args: vec![sort],
            result: "Int".to_owned(),
            role: FunRole::Tag {
                module: module.to_owned(),
                kind: kind.to_owned(),
                variants,
            },
        },
        args: vec![value.clone()],
    }
}

/// The length of a text or a list, `i!len!<Sort>`.
fn len_of(value: &F) -> F {
    let sort = value.sort();
    F::App {
        fun: SmtFun {
            name: format!("i!len!{sort}"),
            args: vec![sort],
            result: "Int".to_owned(),
            role: FunRole::Len,
        },
        args: vec![value.clone()],
    }
}

/// The source name in a function's symbol: the field of `u!f!<field>!...` and
/// `i!f!<field>!...`, the predicate of `u!p!<qualified name>`, and the operation of
/// `i!<op>!<Sort>`.
fn source_symbol(fun: &SmtFun) -> &str {
    let name = fun.name.as_str();
    match fun.role {
        FunRole::Predicate => name.strip_prefix("u!p!").unwrap_or(name),
        FunRole::Field { .. } => name.split('!').nth(2).unwrap_or(name),
        _ => name.split('!').nth(1).unwrap_or(name),
    }
}

/// Arithmetic in QF_UFLIA: multiplication, division and remainder stay linear only with a
/// literal operand (a nonzero divisor); otherwise they are uninterpreted functions, since the
/// logic has no nonlinear terms (`spec/12 §2`, DECISION D-115).
fn arithmetic(op: BinOp, left: F, right: F) -> F {
    let literal = |f: &F| matches!(f, F::Int(_));
    let nonzero = |f: &F| matches!(f, F::Int(d) if d.trim_start_matches('-') != "0");
    let (symbol, name, linear) = match op {
        BinOp::Add => ("+", "add", true),
        BinOp::Sub => ("-", "sub", true),
        BinOp::Mul => ("*", "mul", literal(&left) || literal(&right)),
        BinOp::Div => ("div", "div", nonzero(&right)),
        _ => ("mod", "mod", nonzero(&right)),
    };
    if linear {
        F::op(symbol, vec![left, right])
    } else {
        F::App {
            fun: SmtFun {
                name: format!("i!{name}!Int"),
                args: vec!["Int".to_owned(), "Int".to_owned()],
                result: "Int".to_owned(),
                role: FunRole::Other,
            },
            args: vec![left, right],
        }
    }
}

/// The conjuncts of a formula, for listing.
fn conjuncts(f: &F) -> Vec<&F> {
    match f {
        F::Op { op: "and", args } => args.iter().flat_map(conjuncts).collect(),
        other => vec![other],
    }
}

/// A binder's source name: `x%2` is `x`.
fn source_name(name: &str) -> String {
    match name.split_once('%') {
        Some((source, _)) if !source.is_empty() => source.to_owned(),
        _ => name.to_owned(),
    }
}

/// A refinement's term with `it` written as `value`.
fn instantiate(term: &Term, value: &str) -> String {
    print_term_with(term, &|t| matches!(t, Term::It).then(|| value.to_owned()))
}

/// What declares a refinement, for the related span.
fn clause_role(refinement: &Refinement) -> &'static str {
    if refinement.name.starts_with("in ") {
        "range"
    } else {
        "where clause"
    }
}

/// How a primitive's result shows, from its operands' texts.
fn prim_text(prim: Prim, args: &[String], whole: &str) -> String {
    match (prim, args) {
        (Prim::Len, [a]) => format!("{a}.len"),
        _ => whole.to_owned(),
    }
}

/// `ob-` and the first 12 hex digits of SHA-256 over the module path, the declaration, the
/// kind and the normalized goal (`spec/12 §8`).
fn obligation_id(module: &str, declaration: &str, kind: ObligationKind, goal: &str) -> String {
    let digest =
        Sha256::digest(format!("{module}\n{declaration}\n{}\n{goal}", kind.as_str()).as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("ob-{}", hex.get(..12).unwrap_or(&hex))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(digits: &str) -> F {
        F::Int(digits.to_owned())
    }

    fn var(name: &str, sort: &str) -> F {
        F::Const(Const {
            name: name.to_owned(),
            sort: sort.to_owned(),
            role: ConstRole::Plain,
        })
    }

    /// A package whose user names include `names`: parameters of `f`, fields of its record
    /// kind and kinds of their own, all of them in its clauses, next to a sum whose variants
    /// share a field name with different types and have a field named `tag`.
    fn namespace_program(names: &[String], span: Span) -> Program {
        let int = || base_type(Base::Int);
        let text = || base_type(Base::Text);
        let named = |name: &str| {
            base_type(Base::Named {
                module: "m".to_owned(),
                name: name.to_owned(),
                args: Vec::new(),
            })
        };
        let option = base_type(Base::Named {
            module: una_core::PRELUDE_MODULE.to_owned(),
            name: "Option".to_owned(),
            args: vec![int()],
        });
        let kind = |name: &str| una_core::Path {
            module: "m".to_owned(),
            name: name.to_owned(),
        };
        let var = |name: &str| Term::Var(name.to_owned());
        let eq = |left: Term, right: Term| Term::Binary {
            op: BinOp::Eq,
            left: Box::new(left),
            right: Box::new(right),
        };
        let clause = |term: Term| Clause {
            term,
            text: String::new(),
            span: Some(span),
        };
        // User names that equal the encoder's names, with and without their prefixes.
        let mut user: Vec<String> = [
            "tag",
            "len",
            "lit",
            "path!1",
            "unit",
            "value",
            "failure",
            "div",
            "mod",
            "mul",
            "lt",
            "result",
            "Text",
            "Unit",
            "Hashed",
            "Fn",
            "Unknown",
            "List_Int",
            "Option_Int",
        ]
        .iter()
        .map(|n| (*n).to_owned())
        .collect();
        user.extend(names.iter().cloned());
        let record_fields: Vec<(String, Type)> = user.iter().map(|n| (n.clone(), int())).collect();
        let mut decls = vec![
            Decl::Data {
                name: "K".to_owned(),
                params: Vec::new(),
                variants: vec![
                    DataVariant {
                        name: "Stamp".to_owned(),
                        fields: vec![("tag".to_owned(), int())],
                    },
                    DataVariant {
                        name: "Label".to_owned(),
                        fields: vec![("size".to_owned(), text())],
                    },
                    DataVariant {
                        name: "Circle".to_owned(),
                        fields: vec![("size".to_owned(), int())],
                    },
                ],
                record: false,
            },
            Decl::Data {
                name: "Rec".to_owned(),
                params: Vec::new(),
                variants: vec![DataVariant {
                    name: "Rec".to_owned(),
                    fields: record_fields,
                }],
                record: true,
            },
        ];
        // Predicates named like the encoder's functions, one opaque and one reflect.
        for (name, formula) in [("tag", None), ("len", Some(eq(var("unit"), var("unit"))))] {
            decls.push(Decl::Pred {
                name: name.to_owned(),
                reflect: formula.is_some(),
                params: vec![("unit".to_owned(), int())],
                body: Expr::Atom {
                    atom: Atom::Bool(true),
                    span,
                },
                formula,
                span,
            });
        }
        // Kinds named like the encoder's sorts.
        for name in ["Text", "Unit", "Hashed", "Fn", "Unknown", "Option"] {
            decls.push(Decl::Data {
                name: name.to_owned(),
                params: Vec::new(),
                variants: vec![DataVariant {
                    name: name.to_owned(),
                    fields: Vec::new(),
                }],
                record: false,
            });
        }
        let mut params: Vec<(Name, Type)> = user.iter().map(|n| (n.clone(), int())).collect();
        params.push(("r".to_owned(), named("Rec")));
        params.push(("k".to_owned(), named("K")));
        params.push(("o".to_owned(), option));
        params.push(("s".to_owned(), text()));
        params.push(("h".to_owned(), base_type(Base::Hashed)));
        for name in ["Text", "Unit", "Hashed", "Fn", "Unknown", "Option"] {
            params.push((format!("v{name}"), named(name)));
        }
        let mut requires: Vec<Term> = user.iter().map(|n| eq(var(n), var(n))).collect();
        for (index, name) in user.iter().enumerate() {
            let field = Term::Field {
                value: Box::new(var("r")),
                kind: kind("Rec"),
                field: name.clone(),
                index,
            };
            requires.push(eq(field.clone(), field));
        }
        for name in ["Text", "Unit", "Hashed", "Fn", "Unknown", "Option"] {
            let value = var(&format!("v{name}"));
            requires.push(Term::Is {
                value: Box::new(value),
                ctor: Ctor {
                    kind: kind(name),
                    name: name.to_owned(),
                    index: 0,
                },
            });
        }
        requires.push(eq(Term::Len(Box::new(var("s"))), Term::Int("3".to_owned())));
        requires.push(eq(var("s"), Term::Text("tag".to_owned())));
        requires.push(Term::Binary {
            op: BinOp::Lt,
            left: Box::new(var("s")),
            right: Box::new(Term::Text("lit".to_owned())),
        });
        requires.push(eq(
            Term::Binary {
                op: BinOp::Mul,
                left: Box::new(var("mul")),
                right: Box::new(var("lt")),
            },
            var("mod"),
        ));
        let some = Ctor {
            kind: una_core::Path {
                module: una_core::PRELUDE_MODULE.to_owned(),
                name: "Option".to_owned(),
            },
            name: "Some".to_owned(),
            index: 0,
        };
        let payload = Term::Field {
            value: Box::new(var("o")),
            kind: some.kind.clone(),
            field: "value".to_owned(),
            index: 0,
        };
        requires.push(Term::Is {
            value: Box::new(var("o")),
            ctor: some,
        });
        requires.push(eq(payload.clone(), payload));
        requires.push(eq(var("h"), var("h")));
        for predicate in ["tag", "len"] {
            requires.push(Term::Call {
                predicate: kind(predicate),
                args: vec![var("unit")],
            });
        }
        let arm = |index: usize, bind: &str, body: Expr| una_core::Arm {
            ctor: Ctor {
                kind: kind("K"),
                name: String::new(),
                index,
            },
            binds: vec![Some(bind.to_owned())],
            body,
        };
        let returned = |atom: Atom| Expr::Atom { atom, span };
        let divide = Expr::Let {
            name: "q".to_owned(),
            ty: int(),
            value: Box::new(Bound::Rhs(Rhs::Prim {
                prim: Prim::Div,
                args: vec![Atom::Var("div".to_owned()), Atom::Var("mod".to_owned())],
                arg_spans: vec![span, span],
                span,
            })),
            body: Box::new(Expr::Let {
                name: "u".to_owned(),
                ty: base_type(Base::Unit),
                value: Box::new(Bound::Rhs(Rhs::Atom(Atom::Unit))),
                body: Box::new(returned(Atom::Var("q".to_owned()))),
                span,
                written: false,
            }),
            span,
            written: false,
        };
        let body = Expr::Case {
            scrutinee: Atom::Var("k".to_owned()),
            arms: vec![
                arm(0, "tag%2", returned(Atom::Var("tag%2".to_owned()))),
                arm(1, "size", returned(Atom::Int("1".to_owned()))),
                arm(2, "size%2", divide),
            ],
            otherwise: None,
        };
        let positive = Term::Binary {
            op: BinOp::Gt,
            left: Box::new(Term::Result),
            right: Box::new(Term::Int("0".to_owned())),
        };
        decls.push(Decl::Fun(Fun {
            name: "f".to_owned(),
            type_params: Vec::new(),
            params,
            result: int(),
            uses: Vec::new(),
            fails: None,
            requires: requires.into_iter().map(clause).collect(),
            ensures: vec![clause(positive)],
            body,
            span,
            public: true,
        }));
        Program {
            modules: vec![una_core::Module {
                path: "m".to_owned(),
                decls,
                examples: Vec::new(),
            }],
            ..Program::default()
        }
    }

    /// The symbols a query declares or defines, in order.
    fn declared(query: &str) -> Vec<String> {
        query
            .lines()
            .filter_map(|line| {
                let rest = line
                    .strip_prefix("(declare-sort ")
                    .or_else(|| line.strip_prefix("(declare-fun "))
                    .or_else(|| line.strip_prefix("(define-fun "))?;
                rest.split(' ')
                    .next()
                    .map(|s| s.trim_matches('|').to_owned())
            })
            .collect()
    }

    #[test]
    fn user_names_never_share_a_symbol_with_the_encoder() {
        // ADR 0038 D8 and `spec/12 §4`: user symbols start with `u!` and the encoder's with
        // `i!`. The first program collects the encoder's symbols; the second declares user
        // names equal to every one of them, and no query declares one symbol twice.
        let mut sources = SourceDb::new();
        let file = sources.add("m.una", b"module m\n");
        let span = Span::new(file, 0, 6);
        let first = namespace_program(&[], span);
        let mut internal: BTreeSet<String> = BTreeSet::new();
        let obligations = super::obligations(&first, &sources);
        assert!(obligations.len() >= 2, "{}", obligations.len());
        for obligation in &obligations {
            for symbol in declared(&obligation.query.text()) {
                if symbol.starts_with("i!") {
                    internal.insert(symbol);
                }
            }
        }
        for expected in [
            "i!Text",
            "i!Hashed",
            "i!Unit",
            "i!Option_Int",
            "i!tag!u!Kind_m_K",
            "i!tag!i!Option_Int",
            "i!f!value!i!Option_Int",
            "i!len!i!Text",
            "i!lt!Text",
            "i!mul!Int",
            "i!div!Int",
            "i!lit!746167",
            "i!v!path!1",
            "i!v!unit",
        ] {
            assert!(internal.contains(expected), "{expected} in {internal:?}");
        }
        let names: Vec<String> = internal.iter().cloned().collect();
        let second = namespace_program(&names, span);
        let obligations = super::obligations(&second, &sources);
        let mut users = 0;
        for obligation in &obligations {
            // No symbol stands for two things: a declaration is emitted once per symbol, so two
            // functions or constants sharing one would be merged silently (R-009, R-019).
            let mut meaning: BTreeMap<String, String> = BTreeMap::new();
            for term in obligation.query.applications() {
                let (symbol, what) = match &term {
                    F::App { fun, .. } => (
                        fun.name.clone(),
                        format!("fun {:?} {:?} {:?}", fun.args, fun.result, fun.role),
                    ),
                    F::Const(c) => (c.symbol(), format!("const {} {:?}", c.sort, c.role)),
                    _ => continue,
                };
                if let Some(first) = meaning.insert(symbol.clone(), what.clone()) {
                    assert_eq!(first, what, "{symbol} stands for two things");
                }
            }
            let text = obligation.query.text();
            let symbols = declared(&text);
            let unique: BTreeSet<&String> = symbols.iter().collect();
            assert_eq!(unique.len(), symbols.len(), "{text}");
            for symbol in &symbols {
                assert!(
                    symbol.starts_with("u!") || symbol.starts_with("i!"),
                    "{symbol} has no namespace in {text}"
                );
            }
            users += symbols
                .iter()
                .filter(|s| {
                    names
                        .iter()
                        .any(|n| s.ends_with(n.as_str()) && s.starts_with("u!"))
                })
                .count();
        }
        // Every internal symbol reappears as a user binder or field, in the user namespace.
        assert!(users >= names.len(), "{users} < {}", names.len());
        // Variants sharing the field `size` with two types have two functions (R-019).
        let all: String = obligations.iter().map(|o| o.query.text()).collect();
        assert!(all.contains("u!f!size!Label!u!Kind_m_K"), "{all}");
        assert!(all.contains("u!f!size!Circle!u!Kind_m_K"), "{all}");
        assert!(all.contains("u!f!tag!Stamp!u!Kind_m_K"), "{all}");
    }

    #[test]
    fn a_kind_the_package_declares_is_never_a_prelude_kind() {
        // R-010: tier 0 rejects a module named `prelude` (U1009); the verifier still takes the
        // variants of a kind the package declares from the declaration.
        let variant = |name: &str| DataVariant {
            name: name.to_owned(),
            fields: Vec::new(),
        };
        let program = Program {
            modules: vec![una_core::Module {
                path: una_core::PRELUDE_MODULE.to_owned(),
                decls: vec![Decl::Data {
                    name: "KeyError".to_owned(),
                    params: Vec::new(),
                    variants: vec![
                        variant("UnknownKey"),
                        variant("Revoked"),
                        variant("Expired"),
                    ],
                    record: false,
                }],
                examples: Vec::new(),
            }],
            ..Program::default()
        };
        let index = Index::new(&program);
        assert_eq!(index.variant_names("prelude", "KeyError").len(), 3);
        assert_eq!(index.variant_names("prelude", "FileError").len(), 3);
        let empty = Program::default();
        let index = Index::new(&empty);
        assert_eq!(
            index.variant_names("prelude", "KeyError"),
            vec!["UnknownKey"]
        );
    }

    #[test]
    fn capabilities_are_looked_up_by_qualified_identity() {
        // ADR 0038 D6: a user capability `Files` is not `std.host.Files`, and two capabilities
        // of one name in two modules keep their own contracts.
        let read = |ensures: Vec<Clause>| CapOp {
            name: "read".to_owned(),
            params: vec![("path".to_owned(), base_type(Base::Text))],
            result: base_type(Base::Text),
            fails: None,
            requires: Vec::new(),
            ensures,
            accepts: None,
        };
        let nonempty = Clause {
            term: Term::Bool(true),
            text: "result.len > 0".to_owned(),
            span: None,
        };
        let files = |ensures| Decl::Cap {
            name: "Files".to_owned(),
            ops: vec![read(ensures)],
        };
        let program = Program {
            modules: vec![
                una_core::Module {
                    path: "aaa".to_owned(),
                    decls: vec![files(vec![nonempty])],
                    examples: Vec::new(),
                },
                una_core::Module {
                    path: "bbb".to_owned(),
                    decls: vec![files(Vec::new())],
                    examples: Vec::new(),
                },
            ],
            library: Vec::new(),
            host: vec![files(Vec::new())],
        };
        let index = Index::new(&program);
        let ensures = |module: &str| {
            let path = una_core::Path {
                module: module.to_owned(),
                name: "Files".to_owned(),
            };
            index.operation(&path, "read").map(|op| op.ensures.len())
        };
        assert_eq!(ensures(una_core::HOST_MODULE), Some(0));
        assert_eq!(ensures("aaa"), Some(1));
        assert_eq!(ensures("bbb"), Some(0));
        assert_eq!(ensures("ccc"), None);
    }

    #[test]
    fn formulas_show_as_source_with_the_parentheses_of_the_precedence_table() {
        // `spec/01 §7`: the hypotheses of `una obligations` read as Una.
        let program = Program::default();
        let index = Index::new(&program);
        let sources = SourceDb::new();
        let mut walker = Walker::new(&index, &sources, "m");
        let (a, b, c) = (var("a", "Int"), var("b", "Int"), var("c", "Int"));
        let low = F::op("<=", vec![int("0"), a.clone()]);
        let high = F::op("<", vec![a.clone(), int("20")]);
        walker
            .defs
            .insert("%t0".to_owned(), F::op("+", vec![a.clone(), int("1")]));
        walker
            .temps
            .insert("%t1".to_owned(), "f(x, \"a b\")".to_owned());
        walker.temps.insert("%t2".to_owned(), "g(x) + 1".to_owned());
        let option = kind_sort("prelude", "Option", &["Int".to_owned()]);
        let tag = F::App {
            fun: SmtFun {
                name: format!("tag!{option}"),
                args: vec![option.clone()],
                result: "Int".to_owned(),
                role: FunRole::Tag {
                    module: "prelude".to_owned(),
                    kind: "Option".to_owned(),
                    variants: 2,
                },
            },
            args: vec![var("o", &option)],
        };
        let cases = [
            (
                F::op("*", vec![F::op("+", vec![a.clone(), b.clone()]), int("2")]),
                "(a + b) * 2",
            ),
            (
                F::op("-", vec![a.clone(), F::op("-", vec![b.clone(), c.clone()])]),
                "a - (b - c)",
            ),
            (
                F::op("-", vec![F::op("-", vec![a.clone(), b.clone()]), c.clone()]),
                "a - b - c",
            ),
            (F::op("-", vec![int("-1")]), "-(-1)"),
            (F::not(F::eq(a.clone(), b.clone())), "a != b"),
            (
                F::not(F::op(
                    "ite",
                    vec![low.clone(), high.clone(), F::Bool(false)],
                )),
                "not a in 0..<20",
            ),
            (
                F::op("ite", vec![low.clone(), F::Bool(true), high.clone()]),
                "0 <= a or a < 20",
            ),
            (
                F::not(F::op("ite", vec![low.clone(), F::Bool(true), high.clone()])),
                "not (0 <= a or a < 20)",
            ),
            (
                F::op(
                    "=>",
                    vec![F::op("=>", vec![low.clone(), high.clone()]), low.clone()],
                ),
                "(0 <= a implies a < 20) implies 0 <= a",
            ),
            (
                F::op(
                    "ite",
                    vec![
                        low.clone(),
                        F::op("+", vec![a.clone(), b.clone()]),
                        c.clone(),
                    ],
                ),
                "if 0 <= a then a + b else c",
            ),
            (F::eq(tag.clone(), int("1")), "o is None"),
            (F::not(F::eq(tag, int("0"))), "not o is Some"),
            (F::op("*", vec![var("%t0", "Int"), int("2")]), "(a + 1) * 2"),
            (
                F::op("*", vec![var("%t1", "Int"), var("%t2", "Int")]),
                "f(x, \"a b\") * (g(x) + 1)",
            ),
        ];
        for (formula, shown) in cases {
            assert_eq!(walker.show_formula(&formula), shown);
        }
    }
}
