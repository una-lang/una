//! The environment of the type checker: every kind, alias, function, capability and handler of
//! the package, the prelude and `std.testing`, the scopes of each module (ADR 0023), and the
//! lowering of written types (U2002, U2003).

use std::collections::BTreeMap;
use std::rc::Rc;

use una_core::IntType;
use una_diag::Code;
use una_project::Edition;
use una_syntax::ast::{
    Convention,
    CapabilityDecl, ClauseKind, DeclKind, Expr, Field, File, FnTypeClause, FunctionDecl,
    HandlerDecl, Ident, KindBody, KindDecl, Label as LabelAst, OpClauseKind, Param, PolicyDecl,
    PredicateDecl, Type, TypeCore, UsesClause,
};
use una_syntax::{Interner, Span, Symbol};

use super::Report;
use super::closest;
use super::label::{Accepts, Label};
use super::ty::{
    AliasId, CapId, Declared, FnRefinements, FnTy, FnUse, KindId, KindLabels, Names, Refinement,
    Show, Ty,
};

/// A module the checker knows: a package module, the prelude or `std.testing`.
pub(crate) struct Module<'a> {
    pub(crate) tree: &'a File,
    pub(crate) interner: &'a Interner,
    /// Whether problems in it are reported: a package module without syntax errors (ADR 0023
    /// rule 7). The standard modules are never reported.
    pub(crate) reports: bool,
    /// Its path, from its header.
    pub(crate) path: String,
    pub(crate) upper: BTreeMap<&'a str, Entity>,
    pub(crate) lower: BTreeMap<&'a str, Entity>,
    /// Names imported explicitly.
    pub(crate) imports: BTreeMap<&'a str, Entity>,
    /// Variants of the sum kinds it imports (ADR 0023 rule 2).
    pub(crate) imported_variants: BTreeMap<&'a str, Vec<Entity>>,
    /// What each of `tree.decls` defines, repeats included; `None` for requirements and
    /// examples.
    pub(crate) decls: Vec<Option<Entity>>,
}

/// What a name refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entity {
    Kind(KindId),
    Variant(KindId, usize),
    Alias(AliasId),
    Capability(CapId),
    Function(usize),
    Handler(usize),
    Builtin(Builtin),
}

/// The built-in types (`spec/03 §1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Builtin {
    Int,
    /// A fixed-width integer type, built in from edition 0.2 (`spec/18 §1`).
    Fixed(IntType),
    Bool,
    Text,
    Unit,
    Hashed,
    List,
}

const BUILTINS: [(&str, Builtin); 6] = [
    ("Int", Builtin::Int),
    ("Bool", Builtin::Bool),
    ("Text", Builtin::Text),
    ("Unit", Builtin::Unit),
    ("Hashed", Builtin::Hashed),
    ("List", Builtin::List),
];

/// A field of a record or of a variant.
#[derive(Clone, Debug)]
pub(crate) struct FieldDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) ty: Declared,
    /// Where its type is written.
    pub(crate) span: Span,
}

/// A variant of a sum kind.
#[derive(Clone, Debug)]
pub(crate) struct VariantDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) fields: Vec<FieldDef<'a>>,
}

/// A record or sum kind.
#[derive(Clone, Debug)]
pub(crate) struct KindDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) module: usize,
    pub(crate) params: Vec<&'a str>,
    /// `Some` for a record, with its fields.
    pub(crate) record: Option<Vec<FieldDef<'a>>>,
    /// The variants of a sum.
    pub(crate) variants: Vec<VariantDef<'a>>,
    pub(crate) ast: &'a KindDecl,
}

impl KindDef<'_> {
    pub(crate) fn is_sum(&self) -> bool {
        self.record.is_none()
    }
}

/// An alias or refinement kind, structural over its base (`spec/03 §2` rule 2).
#[derive(Clone, Debug)]
pub(crate) struct AliasDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) module: usize,
    pub(crate) params: Vec<&'a str>,
    pub(crate) base: Declared,
    /// The `where` predicate, over `it` of the base type.
    pub(crate) refinement: Option<&'a Expr>,
    /// The base type as written.
    pub(crate) ty: &'a Type,
}

/// What kind of callable a function definition is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FnKind {
    Function,
    Predicate {
        reflect: bool,
    },
    /// A policy, called as `p(v)` (`spec/09 §3`).
    Policy,
}

/// The declaration of a function definition.
#[derive(Clone, Copy, Debug)]
pub(crate) enum FnAst<'a> {
    Function(&'a FunctionDecl),
    Predicate(&'a PredicateDecl),
    Policy(&'a PolicyDecl),
}

/// A parameter of a function, operation or handler.
#[derive(Clone, Debug)]
pub(crate) struct ParamDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) ty: Declared,
    /// `edit` or `take`, from edition 0.2 (`spec/17 §3`).
    pub(crate) convention: Option<Convention>,
    /// Where its type is written.
    pub(crate) span: Span,
}

/// A capability that a header or a handler requires, `uses @name: C[ops]`, declared or
/// inferred (`spec/05 §3`).
#[derive(Clone, Debug)]
pub(crate) struct Need {
    pub(crate) usage: FnUse,
    /// Where the clause is written, or where an inferred requirement first arises.
    pub(crate) span: Span,
}

/// A function, predicate or policy, with its signature.
#[derive(Clone, Debug)]
pub(crate) struct FnDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) module: usize,
    pub(crate) kind: FnKind,
    pub(crate) type_params: Vec<&'a str>,
    pub(crate) params: Vec<ParamDef<'a>>,
    pub(crate) result: Declared,
    /// Where the result type is written.
    pub(crate) result_span: Span,
    /// The declared error kind (`fails E`); for a policy, that of its `via` function.
    pub(crate) fails: Option<Ty>,
    /// Whether it is `pub`.
    pub(crate) public: bool,
    /// Its `uses`, declared or inferred; for a policy, those of its `via` function.
    pub(crate) uses: Vec<Need>,
    /// Whether its `uses` are inferred: a private function without `uses` (ADR 0007).
    pub(crate) infer_uses: bool,
    /// Whether its `fails` is inferred: a private function without `fails` (ADR 0007).
    pub(crate) infer_fails: bool,
    pub(crate) requires: bool,
    pub(crate) ensures: bool,
    pub(crate) ast: FnAst<'a>,
}

/// An operation of a capability.
#[derive(Clone, Debug)]
pub(crate) struct OpDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) params: Vec<ParamDef<'a>>,
    pub(crate) result: Declared,
    /// Where the result type is written.
    pub(crate) result_span: Span,
    pub(crate) fails: Option<Ty>,
    pub(crate) requires: bool,
    pub(crate) ensures: bool,
    /// The sink label of its arguments (`spec/05 §2`, `spec/07 §4`).
    pub(crate) accepts: Accepts,
    /// The positions of the parameters that accept every label and whose labels the result
    /// carries: `data` of `Keys.hmac` (ADR 0039).
    pub(crate) carried: Vec<usize>,
}

/// A capability.
#[derive(Clone, Debug)]
pub(crate) struct CapDef<'a> {
    pub(crate) name: &'a str,
    pub(crate) module: usize,
    pub(crate) ops: Vec<OpDef<'a>>,
    pub(crate) ast: &'a CapabilityDecl,
}

/// A handler.
#[derive(Clone, Debug)]
pub(crate) struct HandlerDef<'a> {
    pub(crate) module: usize,
    /// Its own `uses`, which the context installing it holds (`spec/05 §4` rule 2).
    pub(crate) uses: Vec<Need>,
    pub(crate) params: Vec<ParamDef<'a>>,
    /// The capability it implements, when its name resolves to one.
    pub(crate) capability: Option<CapId>,
    pub(crate) ast: &'a HandlerDecl,
}

/// A module for [`Env::build`].
pub(crate) struct Source<'a> {
    pub(crate) tree: &'a File,
    pub(crate) interner: &'a Interner,
    pub(crate) reports: bool,
}

/// How far the base of an alias has been lowered, to find aliases defined through themselves.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AliasState {
    Pending,
    Lowering,
    Done,
}

/// The environment.
pub(crate) struct Env<'a> {
    pub(crate) modules: Vec<Module<'a>>,
    pub(crate) kinds: Vec<KindDef<'a>>,
    pub(crate) aliases: Vec<AliasDef<'a>>,
    pub(crate) functions: Vec<FnDef<'a>>,
    pub(crate) capabilities: Vec<CapDef<'a>>,
    pub(crate) handlers: Vec<HandlerDef<'a>>,
    /// The index of the prelude in `modules`; `std.testing` follows it.
    pub(crate) prelude: usize,
    /// The label categories that `una.toml` declares (`spec/07 §2`).
    pub(crate) categories: Vec<String>,
    /// The `via` function of each policy, and the policy (`spec/07 §5`).
    pub(crate) vias: BTreeMap<usize, usize>,
    /// The labels that each kind's fields declare and the variance of its type parameters
    /// (re-review N-001, ADR 0046), computed once the signatures are lowered.
    pub(crate) kind_labels: Rc<KindLabels>,
    alias_state: Vec<AliasState>,
    /// The package's edition (`spec/13 §7`).
    pub(crate) edition: Edition,
}

impl<'a> Env<'a> {
    /// Builds the environment of the package modules, then the prelude and `std.testing`, and
    /// lowers every signature.
    pub(crate) fn build(
        package: Vec<Source<'a>>,
        prelude: Source<'a>,
        testing: Source<'a>,
        categories: Vec<String>,
        edition: Edition,
        report: &mut Report<'_>,
    ) -> Env<'a> {
        let count = package.len();
        let mut env = Env {
            modules: Vec::new(),
            kinds: Vec::new(),
            aliases: Vec::new(),
            functions: Vec::new(),
            capabilities: Vec::new(),
            handlers: Vec::new(),
            prelude: count,
            categories,
            vias: BTreeMap::new(),
            kind_labels: Rc::new(KindLabels::default()),
            alias_state: Vec::new(),
            edition,
        };
        for source in package.into_iter().chain([prelude, testing]) {
            env.register(source);
        }
        if let Some(module) = env.modules.get_mut(count) {
            for (name, builtin) in BUILTINS {
                module.upper.entry(name).or_insert(Entity::Builtin(builtin));
            }
            if edition >= Edition::V0_2 {
                for int in IntType::ALL {
                    module
                        .upper
                        .entry(int.as_str())
                        .or_insert(Entity::Builtin(Builtin::Fixed(int)));
                }
            }
        }
        env.imports(count);
        env.alias_state = vec![AliasState::Pending; env.aliases.len()];
        env.signatures(report);
        for id in 0..env.functions.len() {
            let Some((module, FnAst::Policy(policy))) =
                env.functions.get(id).map(|f| (f.module, f.ast))
            else {
                continue;
            };
            let via = env.text(module, policy.via.name);
            if let Some(Entity::Function(target)) = env.lower(module, via) {
                env.vias.entry(target).or_insert(id);
            }
        }
        let fields: Vec<Vec<&Ty>> = env
            .kinds
            .iter()
            .map(|kind| {
                kind.record
                    .iter()
                    .flatten()
                    .chain(kind.variants.iter().flat_map(|v| v.fields.iter()))
                    .map(|field| &field.ty.base)
                    .collect()
            })
            .collect();
        let params: Vec<usize> = env.kinds.iter().map(|kind| kind.params.len()).collect();
        env.kind_labels = Rc::new(KindLabels::new(&fields, &params));
        env
    }

    /// The text of `symbol`, interned by module `module`.
    pub(crate) fn text(&self, module: usize, symbol: Symbol) -> &'a str {
        match self.modules.get(module) {
            Some(module) => module.interner.resolve(symbol),
            None => "",
        }
    }

    /// Whether problems in module `module` are reported.
    pub(crate) fn reports(&self, module: usize) -> bool {
        self.modules.get(module).is_some_and(|m| m.reports)
    }

    /// Whether module `module` belongs to the package, so its spans can be reported.
    pub(crate) fn in_package(&self, module: usize) -> bool {
        module < self.prelude
    }

    /// Registers the declarations of a module. The first of two declarations with one name
    /// wins, as in ADR 0023 (the second is U1002).
    fn register(&mut self, source: Source<'a>) {
        let index = self.modules.len();
        let interner = source.interner;
        let name = |ident: &Ident| -> &'a str { interner.resolve(ident.name) };
        let path = source
            .tree
            .module
            .as_ref()
            .map_or_else(String::new, |header| {
                let segments: Vec<&str> = header.path.segments.iter().map(name).collect();
                segments.join(".")
            });
        let mut module = Module {
            tree: source.tree,
            interner,
            reports: source.reports,
            path,
            upper: BTreeMap::new(),
            lower: BTreeMap::new(),
            imports: BTreeMap::new(),
            imported_variants: BTreeMap::new(),
            decls: Vec::new(),
        };
        for decl in &source.tree.decls {
            let entity = match &decl.kind {
                DeclKind::Kind(kind) => {
                    let params: Vec<&'a str> = kind.params.iter().map(name).collect();
                    match &kind.body {
                        KindBody::Alias { ty, refinement } => {
                            self.aliases.push(AliasDef {
                                name: name(&kind.name),
                                module: index,
                                params,
                                base: Declared::error(),
                                refinement: refinement.as_deref(),
                                ty,
                            });
                            Some((name(&kind.name), Entity::Alias(self.aliases.len() - 1)))
                        }
                        KindBody::Sum(variants) => {
                            let id = self.kinds.len();
                            self.kinds.push(KindDef {
                                name: name(&kind.name),
                                module: index,
                                params,
                                record: None,
                                variants: variants
                                    .iter()
                                    .map(|variant| VariantDef {
                                        name: name(&variant.name),
                                        fields: Vec::new(),
                                    })
                                    .collect(),
                                ast: kind,
                            });
                            for (position, variant) in variants.iter().enumerate() {
                                module
                                    .upper
                                    .entry(name(&variant.name))
                                    .or_insert(Entity::Variant(id, position));
                            }
                            Some((name(&kind.name), Entity::Kind(id)))
                        }
                        KindBody::Record(_) => {
                            self.kinds.push(KindDef {
                                name: name(&kind.name),
                                module: index,
                                params,
                                record: Some(Vec::new()),
                                variants: Vec::new(),
                                ast: kind,
                            });
                            Some((name(&kind.name), Entity::Kind(self.kinds.len() - 1)))
                        }
                    }
                }
                DeclKind::Capability(capability) => {
                    self.capabilities.push(CapDef {
                        name: name(&capability.name),
                        module: index,
                        ops: Vec::new(),
                        ast: capability,
                    });
                    let id = self.capabilities.len() - 1;
                    Some((name(&capability.name), Entity::Capability(id)))
                }
                DeclKind::Function(function) => {
                    let ast = FnAst::Function(function);
                    let id = self.function(index, name(&function.name), FnKind::Function, ast);
                    Some((name(&function.name), Entity::Function(id)))
                }
                DeclKind::Predicate(predicate) => {
                    let kind = FnKind::Predicate {
                        reflect: predicate.reflect,
                    };
                    let ast = FnAst::Predicate(predicate);
                    let id = self.function(index, name(&predicate.name), kind, ast);
                    Some((name(&predicate.name), Entity::Function(id)))
                }
                DeclKind::Policy(policy) => {
                    let ast = FnAst::Policy(policy);
                    let id = self.function(index, name(&policy.name), FnKind::Policy, ast);
                    Some((name(&policy.name), Entity::Function(id)))
                }
                DeclKind::Handler(handler) => {
                    self.handlers.push(HandlerDef {
                        module: index,
                        uses: Vec::new(),
                        params: Vec::new(),
                        capability: None,
                        ast: handler,
                    });
                    let id = self.handlers.len() - 1;
                    Some((name(&handler.name), Entity::Handler(id)))
                }
                DeclKind::Requirement(_) | DeclKind::Example(_) => None,
            };
            if let Some((text, entity)) = entity {
                let space = match entity {
                    Entity::Function(_) | Entity::Handler(_) => &mut module.lower,
                    _ => &mut module.upper,
                };
                space.entry(text).or_insert(entity);
            }
            module.decls.push(entity.map(|(_, entity)| entity));
        }
        self.modules.push(module);
    }

    fn function(&mut self, module: usize, name: &'a str, kind: FnKind, ast: FnAst<'a>) -> usize {
        let result_span = match ast {
            FnAst::Function(function) => function.result.span,
            FnAst::Predicate(predicate) => predicate.result.span,
            FnAst::Policy(policy) => policy.to.span,
        };
        self.functions.push(FnDef {
            name,
            module,
            kind,
            type_params: Vec::new(),
            params: Vec::new(),
            result: Declared::error(),
            result_span,
            fails: None,
            public: match ast {
                FnAst::Function(function) => function.public,
                FnAst::Predicate(predicate) => predicate.public,
                FnAst::Policy(policy) => policy.public,
            },
            uses: Vec::new(),
            infer_uses: false,
            infer_fails: false,
            requires: false,
            ensures: false,
            ast,
        });
        self.functions.len() - 1
    }

    /// Resolves the imports of the package modules, the first `count` modules. Imports that
    /// resolve to nothing were reported by M3 and are skipped.
    fn imports(&mut self, count: usize) {
        let mut paths: BTreeMap<String, usize> = BTreeMap::new();
        for (index, module) in self.modules.iter().enumerate().take(count) {
            paths.entry(module.path.clone()).or_insert(index);
        }
        paths.insert("std.testing".to_owned(), count + 1);
        for index in 0..count {
            let Some(module) = self.modules.get(index) else {
                continue;
            };
            let mut imports: BTreeMap<&'a str, Entity> = BTreeMap::new();
            let mut variants: BTreeMap<&'a str, Vec<Entity>> = BTreeMap::new();
            for import in &module.tree.imports {
                let path: Vec<&str> = import
                    .module
                    .segments
                    .iter()
                    .map(|segment| module.interner.resolve(segment.name))
                    .collect();
                let Some(target) = paths
                    .get(&path.join("."))
                    .and_then(|&t| self.modules.get(t))
                else {
                    continue;
                };
                for ident in &import.names {
                    let name = module.interner.resolve(ident.name);
                    let Some(&entity) = target.upper.get(name).or_else(|| target.lower.get(name))
                    else {
                        continue;
                    };
                    let own = module.upper.contains_key(name) || module.lower.contains_key(name);
                    if own || imports.contains_key(name) {
                        continue;
                    }
                    imports.insert(name, entity);
                    if let Entity::Kind(kind) = entity
                        && let Some(def) = self.kinds.get(kind)
                    {
                        for (position, variant) in def.variants.iter().enumerate() {
                            variants
                                .entry(variant.name)
                                .or_default()
                                .push(Entity::Variant(kind, position));
                        }
                    }
                }
            }
            if let Some(module) = self.modules.get_mut(index) {
                module.imports = imports;
                module.imported_variants = variants;
            }
        }
    }

    /// The candidates for an upper-case name in `module`, from the innermost scope that holds
    /// any (ADR 0023 rule 2). Several candidates are variants of imported kinds.
    pub(crate) fn upper(&self, module: usize, name: &str) -> Vec<Entity> {
        let Some(scope) = self.modules.get(module) else {
            return Vec::new();
        };
        if let Some(&entity) = scope.upper.get(name).or_else(|| scope.imports.get(name)) {
            return vec![entity];
        }
        if let Some(variants) = scope.imported_variants.get(name) {
            return variants.clone();
        }
        match self
            .modules
            .get(self.prelude)
            .and_then(|p| p.upper.get(name))
        {
            Some(&entity) => vec![entity],
            None => Vec::new(),
        }
    }

    /// The entity of a lower-case name in `module`, outside locals.
    pub(crate) fn lower(&self, module: usize, name: &str) -> Option<Entity> {
        let scope = self.modules.get(module)?;
        scope
            .lower
            .get(name)
            .or_else(|| scope.imports.get(name))
            .or_else(|| {
                self.modules
                    .get(self.prelude)
                    .and_then(|p| p.lower.get(name))
            })
            .copied()
    }

    /// The kind `name` of the prelude, such as `Option` or `Result`.
    pub(crate) fn prelude_kind(&self, name: &str) -> Option<KindId> {
        match self.modules.get(self.prelude)?.upper.get(name)? {
            Entity::Kind(id) => Some(*id),
            _ => None,
        }
    }

    /// The name of a kind, for messages.
    pub(crate) fn kind_name(&self, id: KindId) -> &'a str {
        self.kinds.get(id).map_or("?", |kind| kind.name)
    }

    /// Lowers every signature: aliases, capabilities, kinds, functions and handlers. Capabilities
    /// come before kinds, whose fields may have function types with `uses`.
    fn signatures(&mut self, report: &mut Report<'_>) {
        for id in 0..self.aliases.len() {
            self.lower_alias(id, report);
        }
        for id in 0..self.capabilities.len() {
            self.capability_signature(id, report);
        }
        for id in 0..self.kinds.len() {
            let Some((module, params, ast)) = self
                .kinds
                .get(id)
                .map(|k| (k.module, k.params.clone(), k.ast))
            else {
                continue;
            };
            match &ast.body {
                KindBody::Sum(variants) => {
                    let lowered: Vec<Vec<FieldDef<'a>>> = variants
                        .iter()
                        .map(|variant| self.fields(module, &variant.fields, &params, report))
                        .collect();
                    if let Some(def) = self.kinds.get_mut(id) {
                        for (variant, fields) in def.variants.iter_mut().zip(lowered) {
                            variant.fields = fields;
                        }
                    }
                }
                KindBody::Record(fields) => {
                    let lowered = self.fields(module, fields, &params, report);
                    if let Some(def) = self.kinds.get_mut(id) {
                        def.record = Some(lowered);
                    }
                }
                KindBody::Alias { .. } => {}
            }
        }
        for id in 0..self.functions.len() {
            self.function_signature(id, report);
        }
        // A policy fails and uses capabilities as its `via` function does (`spec/10 §7`).
        for id in 0..self.functions.len() {
            let Some((module, FnAst::Policy(policy))) =
                self.functions.get(id).map(|f| (f.module, f.ast))
            else {
                continue;
            };
            let via = self.text(module, policy.via.name);
            if let Some(Entity::Function(via)) = self.lower(module, via) {
                let (fails, uses) = self
                    .functions
                    .get(via)
                    .map_or((None, Vec::new()), |f| (f.fails.clone(), f.uses.clone()));
                if let Some(def) = self.functions.get_mut(id) {
                    def.fails = fails;
                    def.uses = uses;
                }
            }
        }
        for id in 0..self.handlers.len() {
            let Some((module, ast)) = self.handlers.get(id).map(|h| (h.module, h.ast)) else {
                continue;
            };
            let params = self.params_of(module, &ast.params, &[], Some("a handler"), report);
            let uses: Vec<Need> = first_of_each(
                ast.uses
                    .iter()
                    .map(|uses| self.lower_uses(module, uses, report))
                    .collect(),
            );
            let name = self.text(module, ast.capability.name);
            let capability = self.upper(module, name).into_iter().find_map(|e| match e {
                Entity::Capability(id) => Some(id),
                _ => None,
            });
            if let Some(def) = self.handlers.get_mut(id) {
                def.params = params;
                def.uses = uses;
                def.capability = capability;
            }
        }
    }

    fn capability_signature(&mut self, id: CapId, report: &mut Report<'_>) {
        let Some((module, ast)) = self.capabilities.get(id).map(|c| (c.module, c.ast)) else {
            return;
        };
        let mut ops = Vec::new();
        for op in &ast.ops {
            let fails = op.clauses.iter().find_map(|clause| match &clause.kind {
                OpClauseKind::Fails(kind) => Some(kind),
                _ => None,
            });
            let has = |pick: fn(&OpClauseKind) -> bool| op.clauses.iter().any(|c| pick(&c.kind));
            let op_name = self.text(module, op.name.name);
            let written = op.clauses.iter().find_map(|clause| match &clause.kind {
                OpClauseKind::Accepts(label) => Some(label),
                _ => None,
            });
            // ADR 0039: `Keys.hmac` accepts `public` for `key_name` and every label for
            // `data`, whose label its result carries (`spec/14 §3`). an
            // operation without `accepts` accepts `public`.
            let hmac = module == self.prelude
                && ast_name(self, module, ast) == "Keys"
                && op_name == "hmac";
            let carried: Vec<usize> = if hmac {
                op.params
                    .iter()
                    .position(|p| self.text(module, p.name.name) == "data")
                    .into_iter()
                    .collect()
            } else {
                Vec::new()
            };
            let accepts = match written {
                Some(label) => Some(self.lower_label(module, label, report)),
                None => Some(Label::public()),
            };
            ops.push(OpDef {
                name: self.text(module, op.name.name),
                params: self.params_of(module, &op.params, &[], Some("an operation"), report),
                result: self.lower_type(module, &op.result, &[], report),
                result_span: op.result.span,
                fails: fails.map(|kind| self.error_kind(module, kind, report)),
                requires: has(|kind| matches!(kind, OpClauseKind::Requires(_))),
                ensures: has(|kind| matches!(kind, OpClauseKind::Ensures(_))),
                accepts,
                carried,
            });
        }
        if let Some(def) = self.capabilities.get_mut(id) {
            def.ops = ops;
        }
    }

    fn function_signature(&mut self, id: usize, report: &mut Report<'_>) {
        let Some((module, ast)) = self.functions.get(id).map(|f| (f.module, f.ast)) else {
            return;
        };
        match ast {
            FnAst::Function(function) => {
                let type_params: Vec<&'a str> = function
                    .type_params
                    .iter()
                    .map(|p| self.text(module, p.name))
                    .collect();
                let params = self.params(module, &function.params, &type_params, report);
                let result = self.lower_type(module, &function.result, &type_params, report);
                let fails = function
                    .clauses
                    .iter()
                    .find_map(|clause| match &clause.kind {
                        ClauseKind::Fails(kind) => Some(kind),
                        _ => None,
                    });
                let fails = fails.map(|kind| self.error_kind(module, kind, report));
                let has =
                    |pick: fn(&ClauseKind) -> bool| function.clauses.iter().any(|c| pick(&c.kind));
                let requires = has(|kind| matches!(kind, ClauseKind::Requires(_)));
                let ensures = has(|kind| matches!(kind, ClauseKind::Ensures(_)));
                let declares_uses = has(|kind| matches!(kind, ClauseKind::Uses(_)));
                let declares_fails = has(|kind| matches!(kind, ClauseKind::Fails(_)));
                let uses: Vec<Need> = first_of_each(
                    function
                        .clauses
                        .iter()
                        .filter_map(|clause| match &clause.kind {
                            ClauseKind::Uses(uses) => Some(uses),
                            _ => None,
                        })
                        .map(|uses| self.lower_uses(module, uses, report))
                        .collect(),
                );
                if let Some(def) = self.functions.get_mut(id) {
                    def.type_params = type_params;
                    def.params = params;
                    def.result = result;
                    def.fails = fails;
                    def.requires = requires;
                    def.ensures = ensures;
                    // A private function without `uses` or `fails` infers them.
                    def.infer_uses = !function.public && !declares_uses;
                    def.infer_fails = !function.public && !declares_fails;
                    def.uses = uses;
                }
            }
            FnAst::Predicate(predicate) => {
                let params = self.params_of(module, &predicate.params, &[], Some("a predicate"), report);
                let result = self.lower_type(module, &predicate.result, &[], report);
                if let Some(def) = self.functions.get_mut(id) {
                    def.params = params;
                    def.result = result;
                }
            }
            FnAst::Policy(policy) => {
                let from = self.lower_type(module, &policy.from, &[], report);
                let to = self.lower_type(module, &policy.to, &[], report);
                if let Some(def) = self.functions.get_mut(id) {
                    def.params = vec![ParamDef {
                        name: "",
                        ty: from,
                        convention: None,
                        span: policy.from.span,
                    }];
                    def.result = to;
                }
            }
        }
    }

    fn fields(
        &mut self,
        module: usize,
        fields: &[Field],
        params: &[&'a str],
        report: &mut Report<'_>,
    ) -> Vec<FieldDef<'a>> {
        fields
            .iter()
            .map(|field| FieldDef {
                name: self.text(module, field.name.name),
                ty: self.lower_type(module, &field.ty, params, report),
                span: field.ty.span,
            })
            .collect()
    }

    /// Lowers parameters with `type_params` in scope. Only functions declare conventions:
    /// one on a parameter of a handler, an operation or a predicate is U2020 (`spec/17 §3`).
    pub(crate) fn params(
        &mut self,
        module: usize,
        params: &[Param],
        type_params: &[&'a str],
        report: &mut Report<'_>,
    ) -> Vec<ParamDef<'a>> {
        self.params_of(module, params, type_params, None, report)
    }

    /// [`Env::params`] for `owner`, which names the construct that cannot declare conventions,
    /// or `None` for a function, which can.
    pub(crate) fn params_of(
        &mut self,
        module: usize,
        params: &[Param],
        type_params: &[&'a str],
        owner: Option<&str>,
        report: &mut Report<'_>,
    ) -> Vec<ParamDef<'a>> {
        params
            .iter()
            .map(|param| {
                if let (Some(convention), Some(owner)) = (param.convention, owner)
                    && self.reports(module)
                {
                    report.error(
                        Code::U2020,
                        param.span,
                        format!(
                            "a parameter of {owner} cannot be declared `{}`; only functions declare conventions",
                            convention.as_str()
                        ),
                    );
                }
                ParamDef {
                    name: self.text(module, param.name.name),
                    ty: self.lower_type(module, &param.ty, type_params, report),
                    convention: if owner.is_none() { param.convention } else { None },
                    span: param.ty.span,
                }
            })
            .collect()
    }

    /// `uses @name: C[ops]`: the capability, and the operations it permits, all of them when
    /// none is listed, in the capability's declaration order. A name that is not a capability
    /// is U3001, and an operation that the capability lacks U1001.
    pub(crate) fn lower_uses(
        &mut self,
        module: usize,
        uses: &UsesClause,
        report: &mut Report<'_>,
    ) -> Need {
        let name = self.text(module, uses.cap.name).to_owned();
        let cap_name = self.text(module, uses.cap_type.name.name);
        let reports = self.reports(module);
        let capability = match self.upper(module, cap_name).first().copied() {
            Some(Entity::Capability(id)) => Some(id),
            Some(_) => {
                if reports {
                    report.error(
                        Code::U3001,
                        uses.cap_type.name.span,
                        format!("`{cap_name}` is not a capability"),
                    );
                }
                None
            }
            // An unknown name is U1001, reported by M3.
            None => None,
        };
        let declared: Vec<&'a str> = capability
            .and_then(|id| self.capabilities.get(id))
            .map(|c| c.ops.iter().map(|op| op.name).collect())
            .unwrap_or_default();
        let listed: Vec<&'a str> = uses
            .cap_type
            .ops
            .iter()
            .map(|op| self.text(module, op.name))
            .collect();
        let ops: Vec<String> = if capability.is_none() {
            listed.iter().map(|op| (*op).to_owned()).collect()
        } else if listed.is_empty() {
            declared.iter().map(|op| (*op).to_owned()).collect()
        } else {
            for (op, ident) in listed.iter().zip(&uses.cap_type.ops) {
                if reports && !declared.contains(op) {
                    let mut diagnostic = report.make(
                        Code::U1001,
                        ident.span,
                        format!("capability `{cap_name}` has no operation `{op}`"),
                    );
                    if let Some(close) = closest(op, declared.iter().copied()) {
                        diagnostic.repairs.push(report.edit(
                            format!("write `{close}`"),
                            ident.span,
                            close,
                        ));
                    }
                    report.push(diagnostic);
                }
            }
            declared
                .iter()
                .filter(|op| listed.contains(op))
                .map(|op| (*op).to_owned())
                .collect()
        };
        Need {
            usage: FnUse {
                name,
                capability,
                ops,
            },
            span: uses.span,
        }
    }

    /// The kind named by `fails E`: a sum kind without type parameters (U2002).
    pub(crate) fn error_kind(
        &mut self,
        module: usize,
        ident: &Ident,
        report: &mut Report<'_>,
    ) -> Ty {
        let name = self.text(module, ident.name);
        let problem = match self.upper(module, name).first() {
            // Unknown names are U1001, reported by M3.
            None => return Ty::Error,
            Some(Entity::Kind(id)) => match self.kinds.get(*id) {
                Some(kind) if kind.is_sum() && kind.params.is_empty() => {
                    return Ty::Kind(*id, Vec::new());
                }
                Some(kind) if kind.is_sum() => {
                    format!("error kind `{name}` has type parameters, which `fails` cannot give")
                }
                _ => format!("`fails` names a sum kind, and `{name}` is a record"),
            },
            Some(_) => format!("`fails` names a sum kind, and `{name}` is not one"),
        };
        if self.reports(module) {
            report.error(Code::U2002, ident.span, problem);
        }
        Ty::Error
    }

    /// Lowers the base of an alias; an alias reached again while its own base is lowered is
    /// defined through itself (U2002).
    fn lower_alias(&mut self, id: AliasId, report: &mut Report<'_>) {
        if self.alias_state.get(id) != Some(&AliasState::Pending) {
            return;
        }
        if let Some(state) = self.alias_state.get_mut(id) {
            *state = AliasState::Lowering;
        }
        let Some((module, params, ty)) = self
            .aliases
            .get(id)
            .map(|a| (a.module, a.params.clone(), a.ty))
        else {
            return;
        };
        let base = self.lower_type(module, ty, &params, report);
        // ADR 0045 rules 1 and 2: Core IR has no refinement over a function base, so a `where`
        // clause on a kind whose base is a function type is ill-formed, at the clause. The
        // clause is dropped after the report, so an alias of the kind is not reported again
        // and no refinement is lost later.
        let fn_base = matches!(base.base.unlabeled(), Ty::Fn(_));
        let where_clause = self
            .aliases
            .get(id)
            .and_then(|alias| alias.refinement.map(|expr| (alias.name, expr.span)));
        if fn_base && let Some((name, span)) = where_clause {
            if self.reports(module) {
                report.error(
                    Code::U2002,
                    span,
                    format!(
                        "kind `{name}` refines a function type, which cannot be refined; remove the `where` clause, or refine the parameters or the result of the function type"
                    ),
                );
            }
            if let Some(alias) = self.aliases.get_mut(id) {
                alias.refinement = None;
            }
        }
        if let Some(alias) = self.aliases.get_mut(id) {
            alias.base = base;
        }
        if let Some(state) = self.alias_state.get_mut(id) {
            *state = AliasState::Done;
        }
    }

    /// Lowers a written type of module `module`, with `type_params` in scope. Labels wait for
    /// M6.
    pub(crate) fn lower_type(
        &mut self,
        module: usize,
        ty: &Type,
        type_params: &[&'a str],
        report: &mut Report<'_>,
    ) -> Declared {
        let mut declared = match &ty.core {
            TypeCore::Named { name, args } => {
                let lowered: Vec<Declared> = args
                    .iter()
                    .map(|arg| self.lower_type(module, arg, type_params, report))
                    .collect();
                for (arg, lowered) in args.iter().zip(&lowered) {
                    self.type_argument(module, arg.span, &lowered.base, report);
                    self.refined_argument(module, arg, lowered, type_params, report);
                }
                self.named(module, name, lowered, ty.span, type_params, report)
            }
            TypeCore::List(element) => {
                let lowered = self.lower_type(module, element, type_params, report);
                self.type_argument(module, element.span, &lowered.base, report);
                self.refined_argument(module, element, &lowered, type_params, report);
                Declared::plain(Ty::List(Box::new(lowered.base)))
            }
            TypeCore::Function {
                params,
                result,
                clauses,
            } => {
                // ADR 0037 rule 1: the refinements of the parameters and the result are part
                // of the function type.
                let (params, param_refinements): (Vec<Ty>, Vec<Vec<Refinement>>) = params
                    .iter()
                    .map(|param| {
                        let lowered = self.lower_type(module, param, type_params, report);
                        (lowered.base, lowered.refinements)
                    })
                    .unzip();
                let result = self.lower_type(module, result, type_params, report);
                let refinements = FnRefinements {
                    params: param_refinements,
                    result: result.refinements,
                };
                let result = result.base;
                let fails = clauses.iter().find_map(|clause| match clause {
                    FnTypeClause::Fails(kind) => Some(kind),
                    FnTypeClause::Uses(_) => None,
                });
                let fails = fails.map(|kind| self.error_kind(module, kind, report));
                let clauses: Vec<&UsesClause> = clauses
                    .iter()
                    .filter_map(|clause| match clause {
                        FnTypeClause::Uses(uses) => Some(uses),
                        FnTypeClause::Fails(_) => None,
                    })
                    .collect();
                if self.reports(module) {
                    let names: Vec<(&str, Span)> = clauses
                        .iter()
                        .map(|uses| (self.text(module, uses.cap.name), uses.cap.span))
                        .collect();
                    super::authority::duplicate_refs(report, &names);
                }
                let uses: Vec<FnUse> = first_of_each(
                    clauses
                        .into_iter()
                        .map(|uses| self.lower_uses(module, uses, report))
                        .collect(),
                )
                .into_iter()
                .map(|need| need.usage)
                .collect();
                Declared::plain(Ty::Fn(Box::new(FnTy {
                    params,
                    result,
                    uses,
                    fails,
                    refinements,
                })))
            }
        };
        if let Some(range) = &ty.range {
            // Range sugar on a fixed-width type: its bounds fit the type (`spec/18 §1`).
            if let Ty::Fixed(int) = declared.base.unlabeled()
                && self.reports(module)
            {
                for bound in [&range.low, &range.high] {
                    if !super::fixed::fits(*int, bound.negative, &bound.digits) {
                        let sign = if bound.negative { "-" } else { "" };
                        report.error(
                            Code::U2022,
                            bound.span,
                            format!(
                                "the bound `{sign}{}` does not fit `{}`, which holds `{}` to `{}`",
                                bound.digits,
                                int.as_str(),
                                int.least(),
                                int.greatest()
                            ),
                        );
                    }
                }
            }
            if matches!(
                declared.base.unlabeled(),
                Ty::Int | Ty::Fixed(_) | Ty::Error
            ) {
                let bound = |i: &una_syntax::ast::SignedInt| {
                    format!("{}{}", if i.negative { "-" } else { "" }, i.digits)
                };
                let dots = if range.exclusive { "..<" } else { ".." };
                declared.refinements.push(Refinement::Range(
                    format!("in {}{dots}{}", bound(&range.low), bound(&range.high)),
                    range.span,
                ));
            } else if self.reports(module) {
                let message = if self.edition >= Edition::V0_2 {
                    "range sugar applies only to integer types"
                } else {
                    "range sugar applies only to `Int`"
                };
                report.error(Code::U2002, range.span, message);
            }
        }
        if let Some(label) = &ty.label {
            let label = self.lower_label(module, label, report);
            declared.base = declared.base.with_label(&label);
        }
        declared
    }

    /// U3016: a written type argument that is, or contains, a second-class type, a function
    /// type with `uses` or `fails` (`spec/05 §6`, ADR 0038 D2). DECISION D-142: it is
    /// reported at the innermost
    /// type argument that is a function type, once.
    fn type_argument(&self, module: usize, span: Span, ty: &Ty, report: &mut Report<'_>) {
        if self.reports(module) && matches!(ty.unlabeled(), Ty::Fn(_)) && ty.second_class() {
            report.error(
                Code::U3016,
                span,
                "this type argument is, or holds, a function type with `uses` or `fails`, which is second-class, so it cannot instantiate a type parameter; take the function as a parameter of function type instead",
            );
        }
    }

    /// U2002: a written type argument of a refined kind, or with range sugar (ADR 0040). A type
    /// that contains one is reported at the innermost refined type argument, once, since the
    /// lowered argument keeps only its base.
    fn refined_argument(
        &self,
        module: usize,
        arg: &Type,
        lowered: &Declared,
        type_params: &[&'a str],
        report: &mut Report<'_>,
    ) {
        if !self.reports(module) || !lowered.is_refined() {
            return;
        }
        let base = Show(
            &lowered.base,
            &LowerNames {
                env: self,
                type_params,
            },
        )
        .to_string();
        // N-010: a refined kind is named as written, not by the kind or range its alias goes
        // through.
        let written = match (&arg.core, &arg.range) {
            (TypeCore::Named { name, .. }, None) => Some(self.text(module, name.name)),
            _ => None,
        };
        let what = match (lowered.refinements.last(), written) {
            (None, _) => return,
            (Some(_), Some(name)) => format!("the refined kind `{name}`"),
            (Some(Refinement::Alias(id, _)), None) => {
                let name = self.aliases.get(*id).map_or("?", |a| a.name);
                format!("the refined kind `{name}`")
            }
            (Some(Refinement::Range(range, _)), None) => {
                format!("the refined type `{base} {range}`")
            }
        };
        let mut diagnostic = report.make(
            Code::U2002,
            arg.span,
            format!(
                "this type argument is {what}, and a refined type cannot instantiate a type parameter in v0.1; write its base type `{base}` and check the refinement where the value is used"
            ),
        );
        if arg.label.is_none() {
            diagnostic.repairs.push(report.edit(
                format!("write the base type `{base}`"),
                arg.span,
                base.clone(),
            ));
        }
        report.push(diagnostic);
    }

    /// A written label: its level and categories; a category that `una.toml` does not
    /// declare is U4007 and left out (`spec/07 §2`).
    pub(crate) fn lower_label(
        &self,
        module: usize,
        label: &LabelAst,
        report: &mut Report<'_>,
    ) -> Label {
        let mut lowered = Label {
            level: label.level,
            categories: std::collections::BTreeSet::new(),
            symbols: std::collections::BTreeSet::new(),
        };
        for category in &label.categories {
            let name = self.text(module, category.name);
            if self.categories.iter().any(|declared| declared == name) {
                lowered.categories.insert(name.to_owned());
            } else if self.reports(module) {
                let mut diagnostic = report.make(
                    Code::U4007,
                    category.span,
                    format!(
                        "label category `{name}` is not declared in `[labels] categories` of `una.toml`"
                    ),
                );
                if let Some(close) = closest(name, self.categories.iter().map(String::as_str)) {
                    diagnostic.repairs.push(report.edit(
                        format!("write `{close}`"),
                        category.span,
                        close,
                    ));
                }
                report.push(diagnostic);
            }
        }
        lowered
    }

    /// Lowers `name[args]`.
    fn named(
        &mut self,
        module: usize,
        ident: &Ident,
        args: Vec<Declared>,
        span: Span,
        type_params: &[&'a str],
        report: &mut Report<'_>,
    ) -> Declared {
        let name = self.text(module, ident.name);
        let reports = self.reports(module);
        let mut arity = |expected: usize, found: usize| {
            if reports {
                let takes = match expected {
                    0 => "takes no type arguments".to_owned(),
                    1 => "takes 1 type argument".to_owned(),
                    n => format!("takes {n} type arguments"),
                };
                let given = match found {
                    0 => "none is given".to_owned(),
                    1 => "1 is given".to_owned(),
                    n => format!("{n} are given"),
                };
                report.error(
                    Code::U2003,
                    span,
                    format!("type `{name}` {takes}, but {given}"),
                );
            }
            Declared::error()
        };
        if let Some(index) = type_params.iter().position(|param| *param == name) {
            if !args.is_empty() {
                return arity(0, args.len());
            }
            return Declared::plain(Ty::Param(index));
        }
        let bases: Vec<Ty> = args.into_iter().map(|arg| arg.base).collect();
        let problem = match self.upper(module, name).first().copied() {
            Some(Entity::Builtin(builtin)) => {
                let expected = usize::from(builtin == Builtin::List);
                if bases.len() != expected {
                    return arity(expected, bases.len());
                }
                let base = match builtin {
                    Builtin::Int => Ty::Int,
                    Builtin::Fixed(int) => Ty::Fixed(int),
                    Builtin::Bool => Ty::Bool,
                    Builtin::Text => Ty::Text,
                    Builtin::Unit => Ty::Unit,
                    Builtin::Hashed => Ty::Hashed,
                    Builtin::List => {
                        Ty::List(Box::new(bases.into_iter().next().unwrap_or(Ty::Error)))
                    }
                };
                return Declared::plain(base);
            }
            Some(Entity::Kind(id)) => {
                let expected = self.kinds.get(id).map_or(0, |k| k.params.len());
                if bases.len() != expected {
                    return arity(expected, bases.len());
                }
                return Declared::plain(Ty::Kind(id, bases));
            }
            Some(Entity::Alias(id)) => {
                let expected = self.aliases.get(id).map_or(0, |a| a.params.len());
                if bases.len() != expected {
                    return arity(expected, bases.len());
                }
                return self.alias(id, &bases, ident, reports, report);
            }
            Some(Entity::Variant(kind, _)) => {
                format!(
                    "`{name}` is a variant of `{}`, not a type",
                    self.kind_name(kind)
                )
            }
            Some(Entity::Capability(_)) => {
                format!("`{name}` is a capability, not a type; capabilities are not values")
            }
            // Unknown names are U1001, reported by M3.
            Some(Entity::Function(_) | Entity::Handler(_)) | None => return Declared::error(),
        };
        if reports {
            report.error(Code::U2002, ident.span, problem);
        }
        Declared::error()
    }

    /// The type an alias denotes with arguments `args`: its base, refined by its own predicate
    /// and those of the aliases it goes through (`spec/03 §2` rule 3).
    fn alias(
        &mut self,
        id: AliasId,
        args: &[Ty],
        ident: &Ident,
        reports: bool,
        report: &mut Report<'_>,
    ) -> Declared {
        match self.alias_state.get(id) {
            Some(AliasState::Lowering) => {
                if reports {
                    let name = self.aliases.get(id).map_or("?", |a| a.name);
                    report.error(
                        Code::U2002,
                        ident.span,
                        format!("kind `{name}` is defined in terms of itself"),
                    );
                }
                return Declared::error();
            }
            Some(AliasState::Pending) => self.lower_alias(id, report),
            _ => {}
        }
        let Some(alias) = self.aliases.get(id) else {
            return Declared::error();
        };
        let mut refinements: Vec<Refinement> = alias
            .base
            .refinements
            .iter()
            .map(|refinement| match refinement {
                Refinement::Alias(inner, inner_args) => Refinement::Alias(
                    *inner,
                    inner_args.iter().map(|arg| arg.substitute(args)).collect(),
                ),
                Refinement::Range(text, span) => Refinement::Range(text.clone(), *span),
            })
            .collect();
        if alias.refinement.is_some() {
            refinements.push(Refinement::Alias(id, args.to_vec()));
        }
        Declared {
            base: alias.base.base.substitute(args),
            refinements,
        }
    }
}

/// Names for printing lowered types in messages of the environment.
struct LowerNames<'e, 'a> {
    env: &'e Env<'a>,
    type_params: &'e [&'a str],
}

impl Names for LowerNames<'_, '_> {
    fn kind_name(&self, id: KindId) -> String {
        self.env.kind_name(id).to_owned()
    }

    fn param_name(&self, index: usize) -> String {
        self.type_params
            .get(index)
            .map_or_else(|| "?".to_owned(), |name| (*name).to_owned())
    }
}

/// The first requirement of each name: a reference bound twice is U1002, and the first binding
/// wins, as the first of two declarations does.
fn first_of_each(needs: Vec<Need>) -> Vec<Need> {
    let mut kept: Vec<Need> = Vec::new();
    for need in needs {
        if !kept.iter().any(|k| k.usage.name == need.usage.name) {
            kept.push(need);
        }
    }
    kept
}

/// The name of a capability declaration.
fn ast_name<'a>(env: &Env<'a>, module: usize, ast: &'a CapabilityDecl) -> &'a str {
    env.text(module, ast.name.name)
}
