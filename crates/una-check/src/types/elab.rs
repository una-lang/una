//! Elaboration to Core IR (`spec/09 §3`): the desugaring of the declarations that checked
//! without diagnostics, from what pass 2 resolved, with the formulas and
//! spans that verification needs.

use std::collections::BTreeMap;

use una_core::{
    Arm as CoreArm, Atom, Base, BinOp, Bound, CapOp as CoreCapOp, Clause as CoreClause, Ctor,
    DataVariant, Decl, Example, ExitTag, Expectation, Expr as CoreExpr, FloatOp, Fun,
    HandlerOp as CoreHandlerOp, Label, LoopSource, MapOp, Module as CoreModule, Name, Path, Prim,
    Program, Refinement as CoreRefinement, Rhs, Signature, Source, Term, Type, Use,
};
use una_syntax::ast::{
    Arg, BinaryOp, Block, BlockExpr, Body, CapBindingValue, Clause, ClauseKind, Convention,
    DeclKind, ElseBranch, EverStmt, ExampleDecl, ExpectationKind, Expr, ExprKind, FirstOf,
    ForSource, ForStmt, FunctionDecl, Handled, HandlingKind, IfBlock, IfThenStmt, IsTarget,
    LetValue, Literal, OpClauseKind, PatternKind, Range, SignedInt, StartStmt, Stmt, TasksStmt,
    TextLit, TextPart, TogetherFor, TogetherStmt, UnaryOp, UntilStmt, UsesClause, VariantExpr,
    WhenExpr,
};
use una_syntax::{SourceDb, Span};

use super::Findings;
use super::complex::ComplexMethod;
use super::env::{Constraint, Entity, Env, FnAst, FnDef, FnKind, Need};
use super::records::{ConvertNumber, DenseOp, Records, Res};
use super::ty::{Declared, FnUse, Refinement, Ty};

/// A complex operation whose operands are one complex value, and a factor for `scale`
/// (ADR 0120 rule 23).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComplexStep {
    /// `-x`.
    Neg,
    /// `x.conj()`.
    Conj,
    /// `x.scale(by: k)`.
    Scale,
    /// `x.norm_sqr()`.
    NormSqr,
}

/// What the elaboration of a complex operation at `span` builds with: the complex type, its
/// component type, whether that is `F32`, the label of the result, which every temporary
/// carries, and the record constructor of the complex kind.
struct ComplexParts {
    ty: Type,
    component: Type,
    f32: bool,
    label: Label,
    ctor: Ctor,
    span: Span,
}

/// One binding of a `let` chain being built.
struct Let {
    name: Name,
    ty: Type,
    value: Bound,
    /// The source of the value.
    span: Span,
    /// Whether the source writes the binder's type.
    written: bool,
    /// For `let (x, ...) = a`, the binders of the components; `value` is the atom `a`
    /// (`spec/17 §4`).
    destructure: Option<Vec<(Name, Type)>>,
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
            destructure: None,
        }
    }

    /// `let (x, ...) = tuple` (`spec/17 §4`).
    fn destructure(names: Vec<(Name, Type)>, tuple: Atom, span: Span) -> Let {
        Let {
            name: String::new(),
            ty: unit(),
            value: Bound::Rhs(Rhs::Atom(tuple)),
            span,
            written: false,
            destructure: Some(names),
        }
    }
}

/// What [`program`] could not carry into Core IR, each an internal error (U8001) unless
/// something else rejects the package.
#[derive(Debug, Default)]
pub(crate) struct Gaps {
    /// The components of checker types that a Core IR binder type lost, with the span of
    /// their declaration ([`lost`]).
    pub(crate) lost: Vec<(Span, String)>,
    /// The declarations of the package that type-checked yet have no Core IR, by span, and
    /// what they are (#1050): none of their obligations would be generated.
    pub(crate) missing: Vec<(Span, &'static str)>,
    /// The declarations of the standard modules whose contract, host capability or prelude
    /// kind did not elaborate, by span in the standard sources (#1050): a call into one would
    /// be verified without its `requires`.
    pub(crate) standard: Vec<(Span, &'static str)>,
    /// The positions at which a declaration of the package elaborates to Core expressions
    /// nested deeper than [`una_core::depth::MAX_DEPTH`] (#2264): the declaration is left out
    /// of the program, so that no later pass recurses that deep, and the checker rejects it.
    pub(crate) deep: Vec<Span>,
}

/// Elaborates the package's modules, in path order, and the contracts of the standard
/// library; a declaration with a diagnostic, or with a resolution missing, is left out.
/// Returns the program, and its [`Gaps`]: what Core IR lost of the checked types, and every
/// declaration that type-checked but is missing from it, found by [`missing`] over the program
/// built, whatever path of the elaboration left it out (#1050).
pub(crate) fn program(
    env: &Env<'_>,
    findings: &Findings,
    sources: &SourceDb,
    standard: &SourceDb,
) -> (Program, Gaps) {
    let dbs = Dbs { sources, standard };
    let mut gaps = Gaps::default();
    let mut modules: Vec<CoreModule> = (0..env.prelude)
        .filter(|module| env.reports(*module))
        .map(|module| elaborate_module(env, findings, dbs, module, &mut gaps.lost, &mut gaps.deep))
        .collect();
    if let Some(kinds) = prelude_kinds(env, findings, dbs, &mut gaps.standard) {
        modules.push(kinds);
    }
    modules.extend(standard_kinds(env, findings, dbs, &mut gaps.standard));
    modules.sort_by(|a, b| a.path.cmp(&b.path));
    // The map's `update` is elaborated at each call (ADR 0119 rule 18), so Core declares no
    // signature for it.
    let library = (env.prelude..env.modules.len())
        .filter(|module| Some(*module) != env.map_module)
        .flat_map(|module| library(env, findings, dbs, module, &mut gaps.standard))
        .collect();
    let host = host(env, findings, dbs, &mut gaps.standard);
    let program = Program {
        // ADR 0109 rule 6: the profile is part of the program, which the verifier and the
        // Core export read.
        profile: env.profile,
        modules,
        library,
        host,
        condition_edits: condition_edits(findings),
        // ADR 0069, amendment of 2026-10-19: the frames of a map, in edition `0.5`.
        map_frames: env.edition >= una_project::Edition::V0_5,
    };
    gaps.missing = missing(env, &program);
    (program, gaps)
}

/// What a declaration of the package is in Core IR, and its name there, when it has one: a
/// type, a function (predicates and policies included), a capability, a handler or an
/// example. `None` for a requirement or a unit of measure, which Core IR does not hold.
fn core_key(
    env: &Env<'_>,
    module: usize,
    kind: &DeclKind,
    entity: Option<Entity>,
) -> Option<(&'static str, String)> {
    let key = match (kind, entity) {
        (DeclKind::Example(example), _) => ("example", example_name(example)),
        (DeclKind::Kind(_), Some(Entity::Alias(id))) => (
            "type",
            env.aliases.get(id).map_or("", |a| a.name).to_owned(),
        ),
        (DeclKind::Kind(_), Some(Entity::Kind(id))) => {
            ("type", env.kinds.get(id).map_or("", |k| k.name).to_owned())
        }
        (DeclKind::Kind(kind), _) => ("type", env.text(module, kind.name.name).to_owned()),
        (DeclKind::Function(function), _) => {
            ("function", env.text(module, function.name.name).to_owned())
        }
        (DeclKind::Predicate(predicate), _) => (
            "predicate",
            env.text(module, predicate.name.name).to_owned(),
        ),
        (DeclKind::Policy(policy), _) => ("policy", env.text(module, policy.name.name).to_owned()),
        (DeclKind::Capability(capability), _) => (
            "capability",
            env.text(module, capability.name.name).to_owned(),
        ),
        (DeclKind::Handler(handler), _) => {
            ("handler", env.text(module, handler.name.name).to_owned())
        }
        // A module constant is substituted at each reference (ADR 0126 rule 8): Core IR has no
        // declaration of it.
        (DeclKind::Requirement(_) | DeclKind::Measure(_) | DeclKind::Const(_), _) => return None,
    };
    Some(key)
}

/// The key of a Core IR declaration, as [`core_key`] names the source declaration it comes
/// from.
pub(crate) fn decl_key(decl: &Decl) -> (&'static str, &str) {
    match decl {
        Decl::Alias { name, .. } | Decl::Data { name, .. } => ("type", name),
        Decl::Fun(fun) => ("function", &fun.name),
        Decl::Pred { name, .. } => ("predicate", name),
        Decl::Policy { name, .. } => ("policy", name),
        Decl::Cap { name, .. } => ("capability", name),
        Decl::Handler { name, .. } => ("handler", name),
    }
}

/// The final cross-check of elaboration (#1050): each declaration of a package module that
/// reports, which Core IR holds, must be in the module's Core IR, an example among its
/// examples. Returns the spans of those that are not, with what they are; each is an internal
/// error unless something else rejects the package, never a declaration accepted without its
/// obligations. A declaration with a diagnostic is not in Core IR and is listed too, as that
/// diagnostic rejects the package. Two declarations of a module with one key are counted, and
/// both are listed when Core IR holds fewer. O(d log d) in the d declarations.
pub(crate) fn missing(env: &Env<'_>, program: &Program) -> Vec<(Span, &'static str)> {
    let mut held: BTreeMap<(&str, &'static str, &str), usize> = BTreeMap::new();
    for module in &program.modules {
        for decl in &module.decls {
            let (what, name) = decl_key(decl);
            *held.entry((module.path.as_str(), what, name)).or_default() += 1;
        }
        for example in &module.examples {
            *held
                .entry((module.path.as_str(), "example", example.name.as_str()))
                .or_default() += 1;
        }
    }
    let mut out = Vec::new();
    for module in (0..env.prelude).filter(|module| env.reports(*module)) {
        let Some(m) = env.modules.get(module) else {
            continue;
        };
        let mut wanted: BTreeMap<(&'static str, String), Vec<Span>> = BTreeMap::new();
        for (decl, entity) in m.tree.decls.iter().zip(m.decls.iter().copied()) {
            if let Some(key) = core_key(env, module, &decl.kind, entity) {
                wanted.entry(key).or_default().push(decl.span);
            }
        }
        for ((what, name), spans) in wanted {
            let count = held
                .get(&(m.path.as_str(), what, name.as_str()))
                .copied()
                .unwrap_or(0);
            if count < spans.len() {
                out.extend(spans.into_iter().map(|span| (span, what)));
            }
        }
    }
    out.sort_by_key(|(span, _)| (span.file, span.start, span.end));
    out
}

/// The name of an example, its text without interpolations.
fn example_name(example: &ExampleDecl) -> String {
    example
        .name
        .parts
        .iter()
        .map(|part| match part {
            TextPart::Text(text) => text.clone(),
            TextPart::Interpolation(_) => String::new(),
        })
        .collect()
}

/// The edits of callees' conditions that the package's sites ask for (ADR 0092, amendment of
/// 2026-10-14), in site order: those of a function named as a value anywhere are dropped, by
/// condition (d), and so are the edits of a site, binding and operation that ask for more than
/// one, which the verifier could not tell apart. O(e log e) in the e edits.
fn condition_edits(findings: &Findings) -> Vec<una_core::ConditionEdit> {
    let mut by_site: BTreeMap<(Span, &str, &str), Vec<&una_core::ConditionEdit>> = BTreeMap::new();
    for (target, edit) in &findings.condition_edits {
        if let super::conditions::EditTarget::Function(id) = *target
            && findings.function_values.contains(&id)
        {
            continue;
        }
        by_site
            .entry((edit.site, edit.binding.as_str(), edit.op.as_str()))
            .or_default()
            .push(edit);
    }
    by_site
        .into_values()
        .filter_map(|edits| match edits.as_slice() {
            [first, rest @ ..] if rest.iter().all(|edit| edit == first) => Some((*first).clone()),
            _ => None,
        })
        .collect()
}

/// The Core IR of the package's declarations that a diagnostic left out of [`program`], each
/// with its module's path and its span, in path order, then declaration order (ADR 0091). A
/// declaration whose elaboration is not complete, which lost a component of a checked type or
/// has a resolution missing, is left out, and so are examples. Elaborating reads `findings` and
/// records nothing, so [`program`] and every other pass are unaffected.
pub(crate) fn rejected(
    env: &Env<'_>,
    findings: &Findings,
    sources: &SourceDb,
    standard: &SourceDb,
) -> Vec<super::Rejected> {
    let dbs = Dbs { sources, standard };
    let mut out = Vec::new();
    for module in (0..env.prelude).filter(|module| env.reports(*module)) {
        let Some(m) = env.modules.get(module) else {
            continue;
        };
        for (decl, entity) in m.tree.decls.iter().zip(m.decls.clone()) {
            if !findings.failed.contains(&decl.span) || matches!(decl.kind, DeclKind::Example(_)) {
                continue;
            }
            let Some(entity) = entity else {
                continue;
            };
            let mut fx = Fx::new(env, findings, dbs, module);
            if let Some(core) = fx.decl(&decl.kind, entity)
                && fx.lost.take().is_empty()
            {
                out.push(super::Rejected {
                    module: m.path.clone(),
                    span: decl.span,
                    decl: core,
                });
            }
        }
    }
    out.sort_by(|a, b| a.module.cmp(&b.module));
    out
}

/// The kinds of the prelude that Core IR declares, in the module `prelude`; the others are
/// [`una_core::PRELUDE_ENGINE_KINDS`], which each engine knows itself (#407). `Rounding` and
/// the two complex kinds are edition 0.5's (`spec/29 §16`, `§19`).
pub(crate) const PRELUDE_CORE_KINDS: [&str; 8] = [
    "Json",
    "JsonError",
    "Header",
    "Response",
    "HttpError",
    "Rounding",
    una_core::complex::COMPLEX_F64,
    una_core::complex::COMPLEX_F32,
];

/// The kinds of the server that `std.host` declares in edition 0.4 (ADR 0123 rule 1), which Core
/// IR declares in the module `prelude` only for a package that imports a name of the server, so
/// the Core IR of every other package does not change (`spec/31 §1`).
pub(crate) const SERVER_CORE_KINDS: [&str; 6] = [
    "Method",
    "Param",
    "Request",
    "Exchange",
    "Reply",
    "ServeError",
];

/// The kinds of JSON and their opaque predicate `is_json_number`, which the prelude declares in
/// edition 0.2 (`spec/23 §1`), in a module of the prelude's path: so the verifier generates the
/// obligations of the refinements of their fields, as `Decimal`'s `JsonNumber`, proves them
/// from the facts of `checked`, and both engines read their fields, as for a kind of the
/// package. Both engines call the predicate by name, never its placeholder body. `None` in
/// edition 0.1, where the prelude does not declare them.
/// A kind of [`PRELUDE_CORE_KINDS`] or the predicate that does not elaborate is recorded in
/// `dropped` (#1050).
fn prelude_kinds(
    env: &Env<'_>,
    findings: &Findings,
    dbs: Dbs<'_>,
    dropped: &mut Vec<(Span, &'static str)>,
) -> Option<CoreModule> {
    let m = env.modules.get(env.prelude)?;
    let core_kind = |name: &str| {
        PRELUDE_CORE_KINDS.contains(&name)
            || (env.server_imported && SERVER_CORE_KINDS.contains(&name))
    };
    let mut decls = Vec::new();
    for (decl, entity) in m.tree.decls.iter().zip(m.decls.clone()) {
        let wanted = match &decl.kind {
            DeclKind::Kind(kind) => {
                core_kind(env.text(env.prelude, kind.name.name)).then_some("prelude kind")
            }
            DeclKind::Predicate(predicate) => (env.text(env.prelude, predicate.name.name)
                == "is_json_number")
                .then_some("prelude predicate"),
            _ => None,
        };
        let core = entity.and_then(|entity| {
            let mut fx = Fx::new(env, findings, dbs, env.prelude);
            fx.decl(&decl.kind, entity)
        });
        let Some(core) = core else {
            if let Some(what) = wanted {
                dropped.push((decl.span, what));
            }
            continue;
        };
        let json = match &core {
            Decl::Data { name, .. } => core_kind(name.as_str()),
            Decl::Pred { name, .. } => name == "is_json_number",
            _ => false,
        };
        if json {
            decls.push(core);
        }
    }
    (!decls.is_empty()).then(|| CoreModule {
        path: una_core::PRELUDE_MODULE.to_owned(),
        decls,
        ..CoreModule::default()
    })
}

/// The kinds of the standard modules after `std.testing`, such as `Tick` of `std.drive`
/// (ADR 0116 rule 2), which programs build and read like their own: each such module with its
/// kinds, by path. One that does not elaborate is recorded in `dropped` (#1050).
fn standard_kinds(
    env: &Env<'_>,
    findings: &Findings,
    dbs: Dbs<'_>,
    dropped: &mut Vec<(Span, &'static str)>,
) -> Vec<CoreModule> {
    let mut out = Vec::new();
    for module in env.prelude + 1..env.modules.len() {
        let Some(m) = env.modules.get(module) else {
            continue;
        };
        // `std.testing` has one kind, `TestRequestError` (ADR 0123 rule 21), declared only for a
        // package that imports a name of `std.testing` for servers, so the Core IR of every
        // other package, a server's included, does not change.
        if !m.path.starts_with("std.") || (m.path == "std.testing" && !env.testing_server) {
            continue;
        }
        let mut decls = Vec::new();
        for (decl, entity) in m.tree.decls.iter().zip(m.decls.clone()) {
            // ADR 0117 rule 9: a standard module's capabilities too, as `std.limit`'s `Limit`,
            // whose operations both engines serve by its declaration.
            let what = match &decl.kind {
                DeclKind::Kind(_) => "standard kind",
                DeclKind::Capability(_) => "standard capability",
                _ => continue,
            };
            let core = entity.and_then(|entity| {
                let mut fx = Fx::new(env, findings, dbs, module);
                fx.decl(&decl.kind, entity)
            });
            match core {
                Some(core @ (Decl::Data { .. } | Decl::Cap { .. })) => decls.push(core),
                Some(_) => {}
                None => dropped.push((decl.span, what)),
            }
        }
        if !decls.is_empty() {
            out.push(CoreModule {
                path: m.path.clone(),
                decls,
                ..CoreModule::default()
            });
        }
    }
    out
}

/// The host capabilities, which the prelude declares for `std.host` (`spec/14 §3`). One that
/// does not elaborate is recorded in `dropped` (#1050).
fn host(
    env: &Env<'_>,
    findings: &Findings,
    dbs: Dbs<'_>,
    dropped: &mut Vec<(Span, &'static str)>,
) -> Vec<Decl> {
    let Some(m) = env.modules.get(env.prelude) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (decl, entity) in m.tree.decls.iter().zip(m.decls.clone()) {
        if let DeclKind::Capability(capability) = &decl.kind {
            // ADR 0123 rule 0: `HttpServer` joins Core IR only for a package that imports a
            // name of the server, with the kinds its operations name, so the Core IR of every
            // other package does not change.
            if !env.server_imported
                && env.text(env.prelude, capability.name.name) == super::server::HTTP_SERVER
            {
                continue;
            }
            let core = match entity {
                Some(entity @ Entity::Capability(_)) => {
                    let mut fx = Fx::new(env, findings, dbs, env.prelude);
                    fx.decl(&decl.kind, entity)
                }
                _ => None,
            };
            match core {
                Some(core) => out.push(core),
                None => dropped.push((decl.span, "host capability")),
            }
        }
    }
    out
}

/// The records of module `module`: those of the package, or those of the standard modules,
/// whose spans are those of another source database.
/// The flows the checker recorded (ADR 0103), each with its target's and value's Core IR
/// types, for `una_core::flow::Kinds::pending` to decide which are pending `refinement`
/// obligations, as tier 1 decides (#615). Linear in the flows and their types.
pub(crate) fn refined_flows(
    env: &Env<'_>,
    findings: &Findings,
    sources: &SourceDb,
    standard: &SourceDb,
) -> Vec<(Span, Type, Type)> {
    let dbs = Dbs { sources, standard };
    findings
        .records
        .refined_flows
        .iter()
        .map(|flow| {
            let mut fx = Fx::new(env, findings, dbs, flow.module);
            if let Some(def) = flow.function.and_then(|id| env.functions.get(id)) {
                fx.type_params.clone_from(&def.type_params);
            }
            (
                flow.span,
                fx.convert(&flow.target),
                fx.convert(&flow.source),
            )
        })
        .collect()
}

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
    deep: &mut Vec<Span>,
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
        let mut fx = Fx::new(env, findings, dbs, module);
        // #2264: a declaration whose Core IR nests deeper than the passes after elaboration
        // can recurse is rejected, and left out, so nothing walks it.
        let mut shallow = |exprs: &mut dyn Iterator<Item = &CoreExpr>| {
            let mut found = None;
            for expr in exprs {
                found = una_core::depth::deeper_than(expr, una_core::depth::MAX_DEPTH, decl.span);
                if found.is_some() {
                    break;
                }
            }
            match found {
                Some(span) => {
                    deep.push(span);
                    false
                }
                None => true,
            }
        };
        match (&decl.kind, entity) {
            (DeclKind::Example(example), _) => {
                if let Some(example) = fx.example(example) {
                    let checks = example.expectations.iter().filter_map(|e| match e {
                        Expectation::Holds { check, .. } => Some(check),
                        _ => None,
                    });
                    if shallow(&mut std::iter::once(&example.body).chain(checks)) {
                        out.examples.push(example);
                    }
                }
            }
            (DeclKind::Const(_), Some(Entity::Const(id))) => {
                if let Some(check) = fx.constant_check(id)
                    && shallow(&mut std::iter::once(&check.body))
                {
                    out.constants.push(check);
                }
            }
            (_, Some(entity)) => {
                if let Some(core) = fx.decl(&decl.kind, entity) {
                    let keep = match &core {
                        una_core::Decl::Fun(fun) => shallow(&mut std::iter::once(&fun.body)),
                        una_core::Decl::Pred { body, .. } => shallow(&mut std::iter::once(body)),
                        una_core::Decl::Handler { ops, .. } => {
                            shallow(&mut ops.iter().map(|op| &op.body))
                        }
                        _ => true,
                    };
                    if keep {
                        out.decls.push(core);
                    }
                }
            }
            _ => {}
        }
        lost.extend(fx.lost.take().into_iter().map(|what| (decl.span, what)));
    }
    out
}

/// The signatures of the functions of a standard module. A function whose signature does not
/// elaborate is recorded in `dropped` (#1050): a call into it would otherwise be verified
/// without its contract.
fn library(
    env: &Env<'_>,
    findings: &Findings,
    dbs: Dbs<'_>,
    module: usize,
    dropped: &mut Vec<(Span, &'static str)>,
) -> Vec<Signature> {
    let Some(m) = env.modules.get(module) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (decl, entity) in m.tree.decls.iter().zip(m.decls.clone()) {
        if let DeclKind::Function(function) = &decl.kind {
            // ADR 0123 rule 21: `test_request` and `test_exchange` join Core IR only for a
            // package that imports a name of `std.testing` for servers, so the Core IR of every
            // other package, a server's included, does not change.
            if !env.testing_server
                && m.path == crate::names::prelude::STD_TESTING
                && crate::names::prelude::STD_TESTING_SERVER_NAMES
                    .contains(&env.text(module, function.name.name))
            {
                continue;
            }
            let signature = match entity {
                Some(Entity::Function(id)) => {
                    let mut fx = Fx::new(env, findings, dbs, module);
                    fx.signature(function, id)
                }
                _ => None,
            };
            match signature {
                Some(signature) => out.push(signature),
                None => dropped.push((decl.span, "standard function")),
            }
        }
    }
    out
}

/// The elaboration of one declaration.
struct Fx<'e, 'a> {
    env: &'e Env<'a>,
    records: &'e Records,
    /// The findings of every module, for a clause elaborated in another module's records.
    findings: &'e Findings,
    dbs: Dbs<'e>,
    module: usize,
    type_params: Vec<&'a str>,
    /// Source names in scope and their binders, innermost last.
    scope: Scope,
    /// How many binders each source name has had, for unique names (`spec/09 §1` rule 3).
    used: BTreeMap<String, usize>,
    temps: usize,
    /// The binders of `error`, `it` and `result` where they are allowed.
    error: Option<Name>,
    it: Option<Name>,
    result: Option<Name>,
    /// A resolution was missing: the declaration is not elaborated.
    broken: std::cell::Cell<bool>,
    /// What the Core IR types of this declaration lost of the checker types they come from
    /// (ADR 0038 D1, the consistency check of `spec/10 §2`).
    lost: std::cell::RefCell<Vec<String>>,
    /// The Core type of each binder of an editable binding, for the tuples of the memory
    /// model (`spec/17 §4`).
    types: BTreeMap<Name, Type>,
    /// The `edit` parameters of the function elaborated, in order, and its declared result.
    returning: Option<(Vec<String>, Type)>,
    /// In an `ensures` clause.
    in_ensures: bool,
    /// The wrapping the next block's or line body's tail takes (`spec/17 §4`).
    tail_wrap: Option<Wrap>,
    /// The join of the `if` or `when` being elaborated, which each of its branches takes.
    current_join: Option<Wrap>,
    /// The tuple type of the call at a span, for a call with `edit` arguments.
    call_tuple: Option<(Span, Type)>,
    /// The names of the state of the innermost loop whose body is elaborated, outside any
    /// task in it: what its `skip` and `stop` exit with (`spec/26 §3`).
    loop_names: Option<Vec<String>>,
    /// The module constant whose value is being substituted at a reference, edition 0.5
    /// (ADR 0126 rule 8): its scalar nodes are their folded literals.
    constant: Option<usize>,
    /// Whether the body being elaborated shares each constant that is a record or a list:
    /// elaborated once, at the top of the body, and named by a temporary at every reference,
    /// so a body costs the size of each value it names once, not once per reference.
    sharing: bool,
    /// The temporaries of the shared constants, by constant.
    shared: BTreeMap<usize, Atom>,
    /// The bindings of the shared constants, in dependency order, which the body closes over.
    hoisted: Vec<Let>,
}

/// How a tail value is wrapped into a tuple (`spec/17 §4`).
#[derive(Clone, Debug)]
enum Wrap {
    /// A branch of a construct that updates `names`: `(value, names...)`, the value of `ty`.
    Join { names: Vec<String>, ty: Type },
    /// The body of a function with `edit` parameters: `(params..., result)`.
    Return,
    /// The body of a loop that updates `names`: `(names...)`, its value discarded.
    State { names: Vec<String> },
}

impl<'e, 'a> Fx<'e, 'a> {
    fn new(env: &'e Env<'a>, findings: &'e Findings, dbs: Dbs<'e>, module: usize) -> Self {
        Fx {
            env,
            records: records_of(env, findings, module),
            findings,
            dbs,
            module,
            type_params: Vec::new(),
            scope: Scope::default(),
            used: BTreeMap::new(),
            temps: 0,
            error: None,
            it: None,
            result: None,
            broken: std::cell::Cell::new(false),
            lost: std::cell::RefCell::new(Vec::new()),
            types: BTreeMap::new(),
            returning: None,
            in_ensures: false,
            tail_wrap: None,
            current_join: None,
            loop_names: None,
            call_tuple: None,
            constant: None,
            sharing: false,
            shared: BTreeMap::new(),
            hoisted: Vec::new(),
        }
    }

    /// Elaborates a body with sharing of the constants it names (ADR 0126 rule 8, the
    /// engines' MAY): the values are bound once, before the body. They are closed values, so
    /// binding them first changes no result, and a lambda or a task of the body refers to them
    /// as to any binder in scope.
    fn shared_body(&mut self, elaborate: impl FnOnce(&mut Self) -> CoreExpr) -> CoreExpr {
        let (body, hoisted) = self.sharing(elaborate);
        close(hoisted, body)
    }

    /// Runs `elaborate` with sharing of the constants it names, as [`Fx::shared_body`] does,
    /// and returns its result with the bindings of the shared values, which the caller closes
    /// over the Core expression that names them.
    fn sharing<R>(&mut self, elaborate: impl FnOnce(&mut Self) -> R) -> (R, Vec<Let>) {
        let saved = (
            std::mem::replace(&mut self.sharing, true),
            std::mem::take(&mut self.shared),
            std::mem::take(&mut self.hoisted),
        );
        let result = elaborate(self);
        let hoisted = std::mem::take(&mut self.hoisted);
        (self.sharing, self.shared, self.hoisted) = saved;
        (result, hoisted)
    }

    /// A reference to constant `id` (ADR 0126 rule 8): its value, substituted; when sharing,
    /// a record or a list is elaborated once into the hoisted bindings, and each reference is
    /// its temporary.
    fn constant_atom(&mut self, lets: &mut Vec<Let>, id: usize) -> Atom {
        let scalar = self
            .env
            .constants
            .get(id)
            .is_some_and(|def| self.folded_of(id, def.ast.value.span).is_some());
        if !self.sharing || scalar {
            return self
                .substitute(id, |fx, value| fx.atom(lets, value))
                .unwrap_or(Atom::Unit);
        }
        if let Some(atom) = self.shared.get(&id) {
            return atom.clone();
        }
        let mut own = Vec::new();
        let atom = self
            .substitute(id, |fx, value| fx.atom(&mut own, value))
            .unwrap_or(Atom::Unit);
        self.hoisted.extend(own);
        self.shared.insert(id, atom.clone());
        atom
    }

    /// The folded literal of the node at `span` of constant `id`'s value.
    fn folded_of(&self, id: usize, span: Span) -> Option<Atom> {
        let def = self.env.constants.get(id)?;
        let evaluated = if def.module >= self.env.prelude {
            &self.findings.standard_constants
        } else {
            &self.findings.constants
        };
        evaluated.folded.get(&id)?.get(&span).cloned()
    }

    // ----- module constants (ADR 0126 rule 8) -----

    /// The check of constant `id` (ADR 0126 rule 7): its value, elaborated as a reference
    /// substitutes it, bound to its declared type with the binder written, so the verifier
    /// generates the obligations of the type at the value.
    fn constant_check(&mut self, id: usize) -> Option<una_core::ConstantCheck> {
        let def = self.env.constants.get(id)?;
        // The verifier skips an obligation at a position of a checked value by its span alone,
        // and the standard modules' spans share file ids with the package's: only a package
        // module's constant is ever checked, so every checked span is a package span
        // (#1632 review 2, L2).
        if def.module >= self.env.prelude {
            self.broken.set(true);
            return None;
        }
        let span = def.ast.value.span;
        let mut lets = Vec::new();
        let atom = self.substitute(id, |fx, value| fx.atom(&mut lets, value))?;
        if self.broken.get() {
            return None;
        }
        let ty = self.declared(&def.declared);
        // A temporary, not a source name: a constant is not a binder, so no counterexample
        // shows it, and the goal is closed (ADR 0126 rule 7, R-021).
        let binder = self.temp();
        bind_expr(
            &mut lets,
            binder.clone(),
            ty,
            CoreExpr::Atom { atom, span },
            span,
            true,
        );
        let body = close(
            lets,
            CoreExpr::Atom {
                atom: Atom::Var(binder),
                span,
            },
        );
        Some(una_core::ConstantCheck {
            name: def.name.to_owned(),
            body,
            span,
        })
    }

    /// The folded literal of the node at `span` of the value being substituted.
    fn folded(&self, span: Span) -> Option<Atom> {
        let id = self.constant?;
        let def = self.env.constants.get(id)?;
        let evaluated = if def.module >= self.env.prelude {
            &self.findings.standard_constants
        } else {
            &self.findings.constants
        };
        evaluated.folded.get(&id)?.get(&span).cloned()
    }

    /// Elaborates the value of constant `id` with `elaborate`, in its own module and records,
    /// so that a reference is substituted by the value (ADR 0126 rule 8). `None`, and the
    /// declaration not elaborated, for a constant without a value, which a diagnostic reports.
    fn substitute<R>(
        &mut self,
        id: usize,
        elaborate: impl FnOnce(&mut Self, &Expr) -> R,
    ) -> Option<R> {
        let def = self.env.constants.get(id)?;
        let evaluated = if def.module >= self.env.prelude {
            &self.findings.standard_constants
        } else {
            &self.findings.constants
        };
        if !evaluated.folded.contains_key(&id) {
            self.broken.set(true);
            return None;
        }
        let value = &def.ast.value;
        let saved = (self.module, self.records, self.constant);
        self.module = def.module;
        self.records = records_of(self.env, self.findings, def.module);
        self.constant = Some(id);
        let result = elaborate(self, value);
        (self.module, self.records, self.constant) = saved;
        Some(result)
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
        self.scope.find(name)
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
            // ADR 0104 rule 5: a unit is erased; Core holds the magnitude type.
            Ty::Measured(_, inner) => self.convert(inner),
            // ADR 0103: a refined nested position keeps its refinements in Core IR.
            Ty::Refined(refinements, inner) => self.refined(self.convert(inner), refinements),
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
            // ADR 0119 rule 3: the top label of a collection covers its shape, every label of
            // its items, keys or priorities, the labels their kinds' fields declare included,
            // so that a trace redacts it as a whole.
            Ty::Coll(..) => Type::Base {
                base: self.base(ty),
                refinements: Vec::new(),
                label: implied_label(ty, &self.env.kind_labels),
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
            Ty::Measured(_, inner) => self.base(inner),
            Ty::Int => Base::Int,
            Ty::Fixed(int) => Base::Fixed(*int),
            Ty::Bool => Base::Bool,
            Ty::Text => Base::Text,
            Ty::Unit => Base::Unit,
            Ty::Hashed => Base::Hashed,
            Ty::ReplyRight => Base::ReplyRight,
            Ty::F64 => Base::F64,
            Ty::Decimal(scale) => Base::Decimal(*scale),
            Ty::F32 => Base::F32,
            Ty::List(element) => Base::Named {
                module: "prelude".to_owned(),
                name: "List".to_owned(),
                args: vec![self.convert(element)],
            },
            // ADR 0120: `Array[T]` and `Matrix[T]` are kinds of the prelude in Core, whose sorts
            // the verifier names `i!Array_<T>` and `i!Matrix_<T>`.
            Ty::Dense(dense, element) => Base::Named {
                module: "prelude".to_owned(),
                name: dense.as_str().to_owned(),
                args: vec![self.convert(element)],
            },
            Ty::Map(key, value) => Base::Named {
                module: "prelude".to_owned(),
                name: "Map".to_owned(),
                args: vec![self.convert(key), self.convert(value)],
            },
            // ADR 0119: a collection is named in its module, as a map is in the prelude.
            Ty::Coll(coll, args) => Base::Named {
                module: una_core::COLLECTIONS_MODULE.to_owned(),
                name: coll.as_str().to_owned(),
                args: args.iter().map(|a| self.convert(a)).collect(),
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
            Ty::Labeled(_, inner) | Ty::Refined(_, inner) => self.base(inner),
            Ty::Fn(_) | Ty::Var(_) | Ty::Never | Ty::Error(_) => Base::Unknown,
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
        let mut fx = Fx::new(self.env, self.findings, self.dbs, self.module);
        fx.type_params.clone_from(&self.type_params);
        let core: Vec<CoreRefinement> = refinements
            .iter()
            .filter_map(|refinement| fx.refinement(refinement))
            .collect();
        // A clause that did not elaborate holds a placeholder, never a refinement to keep:
        // the declaration is not elaborated, and what the clause's types lost is this
        // declaration's (#1050).
        if fx.broken.get() {
            self.broken.set(true);
        }
        self.lost.borrow_mut().extend(fx.lost.take());
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
                // The clause is resolved in the alias's module, whose records are those of the
                // standard modules for a kind of the prelude, as `JsonNumber`.
                let records = std::mem::replace(
                    &mut self.records,
                    records_of(self.env, self.findings, alias.module),
                );
                let clause = self.clause(expr);
                self.records = records;
                self.module = module;
                self.scope = scope;
                Some(CoreRefinement {
                    name: alias.name.to_owned(),
                    clause,
                })
            }
        }
    }

    /// The qualified identity of the kind `ty`, when it is one.
    fn kind_path(&self, ty: &Ty) -> Option<Box<Path>> {
        match ty {
            Ty::Kind(id, _) => {
                let def = self.env.kinds.get(*id)?;
                Some(Box::new(self.path(def.module, def.name)))
            }
            Ty::Labeled(_, inner) | Ty::Refined(_, inner) => self.kind_path(inner),
            _ => None,
        }
    }

    fn kind_name(&self, ty: &Ty) -> String {
        match ty {
            Ty::Kind(id, _) => self.env.kind_name(*id).to_owned(),
            Ty::Labeled(_, inner) | Ty::Refined(_, inner) => self.kind_name(inner),
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
                gives: u.gives.clone(),
                conds: Vec::new(),
            })
            .collect()
    }

    /// `uses` with the conditions that `clauses` write on their operations (ADR 0092), lowered
    /// over the parameters in scope, as a `requires` clause is.
    fn with_conditions(&mut self, mut uses: Vec<Use>, clauses: &[&UsesClause]) -> Vec<Use> {
        for clause in clauses {
            let cap = self.name(&clause.cap).to_owned();
            let ops = &clause.cap_type.ops;
            let conds: Vec<(String, CoreClause)> = ops
                .iter()
                .zip(&clause.cap_type.conditions)
                .filter_map(|(op, condition)| {
                    let condition = condition.as_ref()?;
                    Some((self.name(op).to_owned(), self.clause(condition)))
                })
                .collect();
            match uses.iter_mut().find(|u| u.cap == cap) {
                Some(entry) => entry.conds.extend(conds),
                // A written condition with no `uses` entry to carry it would be dropped, and
                // the obligation it puts on the operation's calls with it: fail closed.
                None if !conds.is_empty() => self.broken.set(true),
                None => {}
            }
        }
        uses
    }

    /// The locked conditions of function `id` on the operations its header grants, lowered
    /// over the parameters in scope, with the header's own (ADR 0092 rule 8, as amended).
    fn locks(&mut self, id: usize, def: &FnDef<'a>) -> Vec<una_core::Lock> {
        let Some(entries) = self.env.locks.get(&id).cloned() else {
            return Vec::new();
        };
        let mut locks = Vec::new();
        for entry in &entries {
            let ops = &entry.clause.cap_type.ops;
            for (op, condition) in ops.iter().zip(&entry.clause.cap_type.conditions) {
                let Some(condition) = condition else {
                    continue;
                };
                let op = self.name(op).to_owned();
                let Some(site) = super::lock::header_site(self.env, def, entry, &op) else {
                    continue;
                };
                let (at, header) = (site.at, site.header);
                // (a): only an operation that the header lists by name gets the edit.
                let edit = site
                    .listed
                    .then(|| self.lock_edit(site.insert, header, condition, entry))
                    .flatten();
                // The condition as its string reads, its escapes decoded (#727).
                let mut lock = self.clause(condition);
                if let Some(text) = entry.source.text(condition.span) {
                    text.clone_into(&mut lock.text);
                }
                locks.push(una_core::Lock {
                    cap: entry.binding.to_owned(),
                    header: header.map(|header| self.clause(header)),
                    lock,
                    op,
                    at,
                    entry: entry.span,
                    edit,
                });
            }
        }
        locks
    }

    /// ADR 0092, amendment of 2026-10-15: the edit that writes the condition `condition` of
    /// the lock `entry` back on the header's operation: ` where C_lock` at `insert`, the end of
    /// the operation's `gives` label or, without one, of its name, or `(C_src) and (C_lock)` in
    /// place of its condition `header`. `C_lock` is the text of the
    /// condition's span as the lock's string reads, escapes decoded (#727). `None` under (b),
    /// a condition on two lines, and (d), every conjunct already written; the verifier applies
    /// (c), and the report (e).
    fn lock_edit(
        &self,
        insert: Span,
        header: Option<&Expr>,
        condition: &Expr,
        entry: &super::lock::Entry<'a>,
    ) -> Option<(Span, String)> {
        let locked = entry.source.text(condition.span)?;
        let wanted: Vec<&str> = super::conditions::conjuncts(condition)
            .into_iter()
            .map(|expr| entry.source.text(expr.span))
            .collect::<Option<_>>()?;
        let Some(header) = header else {
            let replacement = super::lock::restoration(locked, &wanted, None)?;
            return Some((insert, replacement));
        };
        let written = self.text(header.span);
        let have: Vec<String> = super::conditions::conjuncts(header)
            .into_iter()
            .map(|expr| self.text(expr.span))
            .collect();
        let have: Vec<&str> = have.iter().map(String::as_str).collect();
        let replacement = super::lock::restoration(locked, &wanted, Some((&written, &have)))?;
        Some((header.span, replacement))
    }

    fn needs(&self, needs: &[Need]) -> Vec<Use> {
        let uses: Vec<FnUse> = needs.iter().map(|n| n.usage.clone()).collect();
        self.uses_of(&uses)
    }

    /// Whether the expression at `span` is an `F64` (`spec/29`).
    fn is_f64_at(&self, span: Span) -> bool {
        matches!(
            self.type_at(span),
            Type::Base {
                base: Base::F64,
                ..
            }
        )
    }

    /// The scale of the expression at `span` when it is a `Decimal[s]` (`spec/29 §16`).
    fn decimal_at(&self, span: Span) -> Option<u8> {
        match self.type_at(span) {
            Type::Base {
                base: Base::Decimal(scale),
                ..
            } => Some(scale),
            _ => None,
        }
    }

    /// Whether the expression at `span` is an `F32` (ADR 0120 rule 5).
    fn is_f32_at(&self, span: Span) -> bool {
        matches!(
            self.type_at(span),
            Type::Base {
                base: Base::F32,
                ..
            }
        )
    }

    /// Whether the expression at `span` is a complex value: `Some(true)` for a `Complex[F32]`,
    /// `Some(false)` for a `Complex[F64]` (ADR 0120 rule 22).
    fn complex_at(&self, span: Span) -> Option<bool> {
        match self.type_at(span) {
            Type::Base {
                base: Base::Named { module, name, .. },
                ..
            } if una_core::complex::is_kind(&module, &name) => {
                Some(name == una_core::complex::COMPLEX_F32)
            }
            _ => None,
        }
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
            self.broken.set(true);
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
            .unwrap_or(Ty::error());
        let args = match record.unlabeled() {
            Ty::Kind(_, args) => args.clone(),
            _ => Vec::new().into(),
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
            self.broken.set(true);
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
                ClauseKind::Ensures(expr) => {
                    self.in_ensures = true;
                    ensures.push(self.clause(expr));
                    self.in_ensures = false;
                }
                ClauseKind::Uses(_)
                | ClauseKind::Fails(_)
                | ClauseKind::UsesOf(_)
                | ClauseKind::FailsOf(_)
                | ClauseKind::Diverges
                | ClauseKind::DerivedFrom(_) => {}
            }
        }
        (requires, ensures)
    }

    /// An expression of the refinement language as a term (`spec/03 §3`).
    /// The checker has already rejected every other form (U2014).
    fn term(&mut self, expr: &Expr) -> Term {
        if let Some(term) = self.folded(expr.span).and_then(atom_term) {
            return term;
        }
        match &expr.kind {
            ExprKind::Paren(inner) => self.term(inner),
            ExprKind::Name(ident)
                if matches!(self.records.res.get(&ident.span), Some(Res::Const(_))) =>
            {
                let Some(Res::Const(id)) = self.records.res.get(&ident.span).cloned() else {
                    return self.outside();
                };
                self.substitute(id, |fx, value| fx.term(value))
                    .unwrap_or(Term::Bool(false))
            }
            // A decimal is its coefficient (`spec/29 §16`, ADR 0120 V2).
            ExprKind::Literal(Literal::Float(text)) if self.decimal_at(expr.span).is_some() => {
                let scale = self.decimal_at(expr.span).unwrap_or(0);
                match una_core::decimal::literal(text, scale) {
                    una_core::decimal::Literal::Value(coefficient) => {
                        Term::Int(coefficient.to_string())
                    }
                    _ => self.outside(),
                }
            }
            ExprKind::Literal(literal) => self.literal_term(literal, expr.span),
            ExprKind::Name(ident) => {
                let name = self.name(ident);
                // In `ensures`, an `edit` parameter is its final value, a component of the
                // result (`spec/17 §3` rule 6, `§4`).
                if let Some(index) = self.ensures_edit(name) {
                    return Term::Proj {
                        value: Box::new(Term::Result),
                        index,
                    };
                }
                #[cfg(test)]
                if hook::eager() {
                    let _ = self.res(ident.span);
                }
                // `res` marks the elaboration broken when nothing is recorded, as for a local:
                // it is asked only for a name that is not one.
                if let Some(binder) = self.local(name) {
                    return Term::Var(binder);
                }
                match self.res(ident.span) {
                    Some(Res::Constant(value)) => Term::Int(value.to_string()),
                    _ => self.outside(),
                }
            }
            ExprKind::Old(inner) => match &inner.kind {
                ExprKind::Name(ident) => match self.local(self.name(ident)) {
                    Some(binder) => Term::Var(binder),
                    None => self.outside(),
                },
                _ => self.outside(),
            },
            ExprKind::UpperName(ident) => match self.res(ident.span) {
                Some(Res::Ctor { kind, variant }) => Term::Ctor(self.ctor(kind, variant)),
                _ => self.outside(),
            },
            ExprKind::Result => match &self.returning {
                Some((edits, _)) if self.in_ensures => Term::Proj {
                    value: Box::new(Term::Result),
                    index: edits.len(),
                },
                _ => Term::Result,
            },
            ExprKind::It => Term::It,
            ExprKind::Binary { op, left, right }
                if (self.is_f64_at(left.span) && self.is_f64_at(right.span))
                    || (self.is_f32_at(left.span) && self.is_f32_at(right.span)) =>
            {
                let f32 = self.is_f32_at(left.span);
                let op = match op {
                    BinaryOp::Add => FloatOp::Add,
                    BinaryOp::Sub => FloatOp::Sub,
                    BinaryOp::Mul => FloatOp::Mul,
                    BinaryOp::Div => FloatOp::Div,
                    BinaryOp::Eq => FloatOp::Eq,
                    BinaryOp::NotEq => FloatOp::Ne,
                    BinaryOp::Lt => FloatOp::Lt,
                    BinaryOp::Le => FloatOp::Le,
                    BinaryOp::Gt => FloatOp::Gt,
                    BinaryOp::Ge => FloatOp::Ge,
                    _ => return self.outside(),
                };
                let args = vec![self.term(left), self.term(right)];
                if f32 {
                    Term::Float32 { op, args }
                } else {
                    Term::Float { op, args }
                }
            }
            ExprKind::Binary { op, left, right } => {
                // Decimal operands of `+`, `-` and the comparisons are aligned by a literal
                // power of ten to the larger scale (ADR 0120 V2); `*` multiplies coefficients.
                let (left_term, right_term) = (self.term(left), self.term(right));
                let (left_term, right_term) =
                    match (self.decimal_at(left.span), self.decimal_at(right.span)) {
                        (Some(a), Some(b)) if *op != BinaryOp::Mul => {
                            let m = a.max(b);
                            (aligned(left_term, m - a), aligned(right_term, m - b))
                        }
                        _ => (left_term, right_term),
                    };
                Term::Binary {
                    op: bin_op(*op),
                    left: Box::new(left_term),
                    right: Box::new(right_term),
                }
            }
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } if self.is_f64_at(operand.span) => Term::Float {
                op: FloatOp::Neg,
                args: vec![self.term(operand)],
            },
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } if self.is_f32_at(operand.span) => Term::Float32 {
                op: FloatOp::Neg,
                args: vec![self.term(operand)],
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
                        right: Box::new(self.literal_term(literal, expr.span)),
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
                // `b.size` of a bit set (ADR 0119 V5).
                Some(Res::BitSetMethod(una_core::BitSetOp::Size)) => {
                    Term::Size(Box::new(self.term(target)))
                }
                // `m.rows` and `m.columns` (`spec/29 §17`).
                Some(Res::Shape(fun)) => Term::Shape {
                    fun,
                    value: Box::new(self.term(target)),
                },
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
                // ADR 0104 rule 4: `magnitude()` and `measured()` are the identity on the
                // magnitude, `convert()` the multiplication by its literal ratio.
                Some(Res::UnitMethod { ratio: 1 }) => self.term(target),
                Some(Res::UnitMethod { ratio }) => Term::Binary {
                    op: BinOp::Mul,
                    left: Box::new(self.term(target)),
                    right: Box::new(Term::Int(ratio.to_string())),
                },
                // ADR 0114 rule 12.7 and ADR 0120 rule 65: on a float, the product or quotient
                // by the float literal of the ratio; on a decimal, its coefficient times the
                // ratio.
                Some(Res::Convert {
                    ratio,
                    divide,
                    number,
                }) => {
                    let value = self.term(target);
                    let op = if divide { FloatOp::Div } else { FloatOp::Mul };
                    match number {
                        ConvertNumber::F64 => Term::Float {
                            op,
                            args: vec![value, Term::F64(float_ratio(ratio).to_bits())],
                        },
                        ConvertNumber::F32 => Term::Float32 {
                            op,
                            args: vec![value, Term::F32(float32_ratio(ratio).to_bits())],
                        },
                        ConvertNumber::Decimal(_) => Term::Binary {
                            op: BinOp::Mul,
                            left: Box::new(value),
                            right: Box::new(Term::Int(ratio.to_string())),
                        },
                    }
                }
                // `x.coefficient()` is the decimal's own term (ADR 0120 V2).
                Some(Res::IntMethod(Prim::Decimal(una_core::DecimalOp::Coefficient))) => {
                    self.term(target)
                }
                // `n.to_f64()` of an integer (ADR 0114 rule 8.4), with its type's width.
                Some(Res::IntMethod(Prim::Float(FloatOp::FromInt))) => Term::ToF64 {
                    value: Box::new(self.term(target)),
                    from: match self.type_at(target.span) {
                        Type::Base {
                            base: Base::Fixed(int),
                            ..
                        } => Some(int),
                        _ => None,
                    },
                },
                // `n.to_f32()` of an integer (ADR 0120 V1), with its type's width.
                Some(Res::IntMethod(Prim::Float32(FloatOp::FromInt))) => Term::ToF32 {
                    value: Box::new(self.term(target)),
                    from: match self.type_at(target.span) {
                        Type::Base {
                            base: Base::Fixed(int),
                            ..
                        } => Some(int),
                        _ => None,
                    },
                },
                // A method of `F64` of the refinement language (ADR 0114 rule 8.4), `to_f32()`
                // of an `F64` included (ADR 0120 V1).
                Some(Res::IntMethod(Prim::Float(op))) if op.in_refinement_language() => {
                    let mut terms = vec![self.term(target)];
                    terms.extend(args.iter().map(|arg| self.term(&arg.value)));
                    Term::Float { op, args: terms }
                }
                // The element of a dense value at its indices (`spec/29 §17`).
                Some(Res::DenseElement(order)) => {
                    let value = Box::new(self.term(target));
                    let mut indices = Vec::new();
                    for position in order {
                        match args.get(position) {
                            Some(arg) => indices.push(self.term(&arg.value)),
                            None => return self.outside(),
                        }
                    }
                    Term::Element {
                        value,
                        indices,
                        tensor: false,
                    }
                }
                // The element of a tensor at the indices of a list literal (`spec/29 §17`).
                Some(Res::TensorElement(position)) => {
                    let value = Box::new(self.term(target));
                    let Some(ExprKind::List(items)) = args.get(position).map(|arg| &arg.value.kind)
                    else {
                        return self.outside();
                    };
                    let indices = items.iter().map(|item| self.term(item)).collect();
                    Term::Element {
                        value,
                        indices,
                        tensor: true,
                    }
                }
                // `t.dim(axis: k)` (`spec/29 §17`).
                Some(Res::Dim(position)) => match args.get(position) {
                    Some(axis) => Term::Dim {
                        value: Box::new(self.term(target)),
                        axis: Box::new(self.term(&axis.value)),
                    },
                    None => self.outside(),
                },
                // A method of `F32` of the refinement language, `to_f64()` included.
                Some(Res::IntMethod(Prim::Float32(op))) if op.in_refinement_language() => {
                    let mut terms = vec![self.term(target)];
                    terms.extend(args.iter().map(|arg| self.term(&arg.value)));
                    Term::Float32 { op, args: terms }
                }
                // `x.complex(im: y)` (ADR 0120 V3).
                Some(Res::Ctor { kind, variant }) => {
                    let receiver = self.term(target);
                    let parts = self.ordered_terms(expr.span, Some(receiver), args);
                    self.complex_term(kind, variant, parts)
                }
                // `s.has(item: x)` of a set, a hash set or a bit set (ADR 0119 V5).
                Some(
                    Res::SetMethod(una_core::SetOp::Has)
                    | Res::HashSetMethod(una_core::HashSetOp::Has)
                    | Res::BitSetMethod(una_core::BitSetOp::Has),
                ) => match args.first() {
                    Some(item) => Term::Member {
                        set: Box::new(self.term(target)),
                        item: Box::new(self.term(&item.value)),
                    },
                    None => self.outside(),
                },
                // `m.has(key: k)` of a map (`spec/21 §4` rule 4), and of a hash map (ADR 0119
                // V5).
                Some(Res::MapMethod(MapOp::Has) | Res::HashMapMethod(una_core::HashMapOp::Has)) => {
                    match args.first() {
                        Some(key) => Term::Has {
                            map: Box::new(self.term(target)),
                            key: Box::new(self.term(&key.value)),
                        },
                        None => self.outside(),
                    }
                }
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
                // `nan()` and `infinity()` are constants (ADR 0114 rule 8.4).
                Some(Res::Function { id, .. })
                    if self.env.functions.get(id).is_some_and(|f| {
                        f.module == self.env.prelude && matches!(f.name, "nan" | "infinity")
                    }) =>
                {
                    let infinity = self
                        .env
                        .functions
                        .get(id)
                        .is_some_and(|f| f.name == "infinity");
                    Term::F64(if infinity {
                        f64::INFINITY.to_bits()
                    } else {
                        una_core::float::CANONICAL_NAN
                    })
                }
                Some(Res::Function { id, .. }) => {
                    let args = self.ordered_terms(expr.span, None, args);
                    Term::Call {
                        predicate: self.function_path(id),
                        args,
                    }
                }
                // `complex(re: x, im: y)` (ADR 0120 V3).
                Some(Res::Ctor { kind, variant }) => {
                    let parts = self.ordered_terms(expr.span, None, args);
                    self.complex_term(kind, variant, parts)
                }
                _ => self.outside(),
            },
            _ => self.outside(),
        }
    }

    /// `complex(re: a, im: b)` as a term, from its components in parameter order (ADR 0120
    /// V3); the checker admits no other construction in a clause.
    fn complex_term(&mut self, kind: usize, variant: Option<usize>, parts: Vec<Term>) -> Term {
        let ctor = self.ctor(kind, variant);
        if !una_core::complex::is_kind(&ctor.kind.module, &ctor.kind.name) {
            return self.outside();
        }
        let mut parts = parts.into_iter();
        match (parts.next(), parts.next(), parts.next()) {
            (Some(re), Some(im), None) => Term::Complex {
                ctor,
                re: Box::new(re),
                im: Box::new(im),
            },
            _ => self.outside(),
        }
    }

    fn literal_term(&mut self, literal: &Literal, span: Span) -> Term {
        match literal {
            Literal::Int(digits) => Term::Int(digits.clone()),
            // A float literal by its exact bits, at the type it took (ADR 0120 rule 6).
            Literal::Float(text) if self.is_f32_at(span) => {
                match una_core::float32::literal(text) {
                    una_core::float32::Literal::Value(value) => Term::F32(value.to_bits()),
                    _ => self.outside(),
                }
            }
            Literal::Float(text) => match una_core::float::literal(text) {
                una_core::float::Literal::Value(value) => Term::F64(value.to_bits()),
                _ => self.outside(),
            },
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
        self.broken.set(true);
        Term::Bool(false)
    }

    /// The arguments of the call at `span` as terms, in parameter order. Every parameter has
    /// its argument, so a parameter with none, or a call whose list literal the checker spread
    /// over several parameters (`spec/29 §17`, a call of a prelude function that no clause
    /// holds, U2014), leaves the declaration not elaborated rather than shifting the arguments
    /// after the hole.
    fn ordered_terms(&mut self, span: Span, receiver: Option<Term>, args: &[Arg]) -> Vec<Term> {
        if self.records.spread.contains_key(&span) {
            self.broken.set(true);
        }
        let order = self.order(span);
        let mut written: Vec<Term> = receiver.into_iter().collect();
        for arg in args {
            written.push(self.term(&arg.value));
        }
        if written.len() != order.len() {
            self.broken.set(true);
        }
        let count = order.iter().flatten().max().map_or(0, |max| max + 1);
        let mut slots: Vec<Option<Term>> = vec![None; count.max(written.len())];
        for (value, target) in written.into_iter().zip(order.iter().copied()) {
            match target.and_then(|t| slots.get_mut(t)) {
                Some(slot) => *slot = Some(value),
                None => self.broken.set(true),
            }
        }
        if slots.iter().any(Option::is_none) {
            self.broken.set(true);
        }
        slots.into_iter().flatten().collect()
    }

    // ----- declarations -----

    /// The contract of a function of a standard module.
    fn signature(&mut self, function: &'a FunctionDecl, id: usize) -> Option<Signature> {
        let def = self.env.functions.get(id)?;
        self.type_params = def.type_params.clone();
        let params = self.params(&def.params);
        let declared_result = self.declared(&def.result);
        let result = self.returning(&def.params, &params, declared_result);
        let (requires, ensures) = self.contracts(&function.clauses);
        self.returning = None;
        #[cfg(test)]
        if hook::breaks(def.name) {
            self.broken.set(true);
        }
        (!self.broken.get()).then(|| Signature {
            path: self.path(def.module, def.name),
            type_params: def.type_params.iter().map(|p| (*p).to_owned()).collect(),
            constraints: core_constraints(&def.constraints, def.type_params.len()),
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
                let declared_result = self.declared(&def.result);
                let result = self.returning(&def.params, &params, declared_result);
                let (requires, ensures) = self.contracts(&function.clauses);
                let written: Vec<&UsesClause> = function
                    .clauses
                    .iter()
                    .filter_map(|clause| match &clause.kind {
                        ClauseKind::Uses(uses) => Some(uses),
                        _ => None,
                    })
                    .collect();
                let uses = self.needs(&def.uses);
                let uses = self.with_conditions(uses, &written);
                if self.returning.is_some() {
                    self.tail_wrap = Some(Wrap::Return);
                }
                let body = self.shared_body(|fx| fx.body(&function.body));
                self.returning = None;
                self.tail_wrap = None;
                let uses_of = def
                    .uses_of
                    .iter()
                    .filter_map(|&index| params.get(index).map(|(name, _)| name.clone()))
                    .collect();
                let locks = self.locks(id, def);
                Decl::Fun(Box::new(Fun {
                    name: def.name.to_owned(),
                    type_params: def.type_params.iter().map(|p| (*p).to_owned()).collect(),
                    constraints: core_constraints(&def.constraints, def.type_params.len()),
                    params,
                    result,
                    uses,
                    uses_of,
                    locks,
                    fails: def.fails.as_ref().map(|f| self.kind_name(f)),
                    error: def.fails.as_ref().and_then(|f| self.kind_path(f)),
                    requires,
                    ensures,
                    body,
                    span: function.name.span,
                    public: function.public,
                }))
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
                        error: op.fails.as_ref().and_then(|f| self.kind_path(f)),
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
                // Conditions speak of the handler's parameters only (ADR 0092 rule 1).
                let written: Vec<&UsesClause> = handler.uses.iter().collect();
                let uses = self.needs(&def.uses);
                let uses = self.with_conditions(uses, &written);
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
                    let body = self.shared_body(|fx| fx.body(&op.body));
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
                    uses,
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
        #[cfg(test)]
        if hook::breaks(decl_key(&decl).1) {
            self.broken.set(true);
        }
        (!self.broken.get()).then_some(decl)
    }

    fn params(&mut self, params: &[super::env::ParamDef<'a>]) -> Vec<(Name, Type)> {
        params
            .iter()
            .map(|p| {
                let ty = self.declared(&p.ty);
                let binder = self.bind(p.name);
                self.types.insert(binder.clone(), ty.clone());
                (binder, ty)
            })
            .collect()
    }

    fn example(&mut self, example: &'a ExampleDecl) -> Option<Example> {
        let name = example_name(example);
        #[cfg(test)]
        if hook::breaks(&name) {
            self.broken.set(true);
        }
        // The `given` lines and the `do` line share the constants they name, in one Core
        // expression, as a function body does (ADR 0126 rule 8, #1632 review 2, #2264): the
        // values are bound once, before the handlers are installed.
        let (body, hoisted) = self.sharing(|fx| fx.example_body(example));
        let body = close(hoisted, body?);
        let result = "result".to_owned();
        let mut expectations = Vec::new();
        for expectation in &example.expectations {
            let text = self.text(expectation.span);
            let line = self.dbs.sources.location(expectation.span).start.line;
            expectations.push(match &expectation.kind {
                ExpectationKind::Holds(expr) => {
                    let outer = self.result.replace(result.clone());
                    // Each `expect` line is a Core expression of its own, which shares the
                    // constants it names (#2264).
                    let check = self.shared_body(|fx| fx.expr(expr));
                    self.result = outer;
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
        (!self.broken.get()).then_some(Example {
            name,
            body,
            expectations,
            stop_after: example.stop_after.as_ref().map(|line| line.iterations),
        })
    }

    /// The `given` lines and the `do` line of a test, as one Core expression: the handlers'
    /// arguments, then the handlers installed around the `do` line. `None` when a `given`
    /// line names no handler.
    fn example_body(&mut self, example: &'a ExampleDecl) -> Option<CoreExpr> {
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
                    span: given.handler.span,
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
        Some(close(lets, body))
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
            Body::Line(expr) => match self.tail_wrap.take() {
                Some(Wrap::Return) => {
                    let saved = self.current_join.take();
                    let value = self.return_line(expr);
                    self.current_join = saved;
                    value
                }
                Some(wrap) => {
                    let saved = self.current_join.take();
                    let value = self.expr(expr);
                    self.current_join = saved;
                    let mut lets = Vec::new();
                    let tail = self.wrap_tail(&mut lets, value, wrap, expr.span);
                    close(lets, tail)
                }
                None => self.expr(expr),
            },
            Body::Block(block) => self.block(block),
        }
    }

    fn block(&mut self, block: &Block) -> CoreExpr {
        // The wrapping of this block's tail, and no join for its own statements (`spec/17
        // §4`).
        let wrap = self.tail_wrap.take();
        let join = self.current_join.take();
        let mark = self.scope.len();
        let mut lets: Vec<Let> = Vec::new();
        let last = block.stmts.len().saturating_sub(1);
        let mut tail = CoreExpr::Atom {
            atom: Atom::Unit,
            span: block.span,
        };
        let mut tail_span = block.span;
        // A function's result returned by a branching last line is wrapped in each branch,
        // at the value each one returns (`spec/12 §3`).
        let returning = matches!(wrap, Some(Wrap::Return));
        let mut wrapped = false;
        for (index, stmt) in block.stmts.iter().enumerate() {
            match stmt {
                // `stop` and `skip` end the block's path with the loop's state; nothing after
                // them in the block runs (`spec/26 §3`).
                Stmt::Stop(stmt) if stmt.condition.is_none() => {
                    tail = self.exit(ExitTag::Stop, stmt.span);
                    wrapped = true;
                    break;
                }
                Stmt::Skip(span) => {
                    tail = self.exit(ExitTag::Next, *span);
                    wrapped = true;
                    break;
                }
                Stmt::Ever(stmt) => {
                    self.ever_loop(&mut lets, stmt);
                    if index == last {
                        tail = CoreExpr::Atom {
                            atom: Atom::Unit,
                            span: stmt.span,
                        };
                        tail_span = stmt.span;
                    }
                }
                // U2034, already reported.
                Stmt::OnStop(_) => self.broken.set(true),
                Stmt::Until(stmt) => {
                    self.until_loop(&mut lets, stmt);
                    if index == last {
                        tail = CoreExpr::Atom {
                            atom: Atom::Unit,
                            span: stmt.span,
                        };
                        tail_span = stmt.span;
                    }
                }
                // `stop when c` is `if c` around `stop`.
                Stmt::Stop(stmt) => {
                    let cond = match &stmt.condition {
                        Some(condition) => self.atom(&mut lets, condition),
                        None => Atom::Bool(true),
                    };
                    let expr = CoreExpr::If {
                        cond,
                        then: Box::new(self.exit(ExitTag::Stop, stmt.span)),
                        otherwise: Box::new(CoreExpr::Atom {
                            atom: Atom::Unit,
                            span: stmt.span,
                        }),
                    };
                    bind_expr(&mut lets, "_".to_owned(), unit(), expr, stmt.span, false);
                    if index == last {
                        tail = CoreExpr::Atom {
                            atom: Atom::Unit,
                            span: stmt.span,
                        };
                        tail_span = stmt.span;
                    }
                }
                // A `together for` and a `first of` are wrapped as a whole, after their tasks
                // (`spec/19 §5`, `spec/27 §6`).
                Stmt::BlockExpr(expr)
                    if index == last
                        && returning
                        && !matches!(expr, BlockExpr::TogetherFor(_) | BlockExpr::FirstOf(_)) =>
                {
                    let saved = self.current_join.replace(Wrap::Return);
                    tail = self.block_expr(expr);
                    self.current_join = saved;
                    wrapped = true;
                }
                Stmt::Handled(handled)
                    if index == last
                        && returning
                        && handled.with.is_empty()
                        && handled.within.is_none()
                        && handled.handlings.is_empty()
                        && !self.marked(&handled.expr) =>
                {
                    tail = self.return_line(&handled.expr);
                    wrapped = true;
                }
                // As the last line, the value is `unit`, returned at the statement.
                Stmt::Assign(stmt) => {
                    let (value, span) = self.value(&mut lets, &stmt.value);
                    self.assign(&mut lets, &stmt.place, value, span);
                    if index == last {
                        tail = CoreExpr::Atom {
                            atom: Atom::Unit,
                            span: stmt.span,
                        };
                        tail_span = stmt.span;
                    }
                }
                // Never the last line of a block of a checked program: the parser or U2024
                // reports it.
                Stmt::Together(stmt) => self.together(&mut lets, stmt),
                // As the last line, the value is `unit` (`spec/27 §7`).
                Stmt::Tasks(stmt) => {
                    self.tasks(&mut lets, stmt);
                    if index == last {
                        tail = CoreExpr::Atom {
                            atom: Atom::Unit,
                            span: stmt.span,
                        };
                        tail_span = stmt.span;
                    }
                }
                Stmt::Start(stmt) => {
                    self.start(&mut lets, stmt);
                    if index == last {
                        tail = CoreExpr::Atom {
                            atom: Atom::Unit,
                            span: stmt.span,
                        };
                        tail_span = stmt.span;
                    }
                }
                Stmt::For(stmt) => {
                    self.for_loop(&mut lets, stmt);
                    if index == last {
                        tail = CoreExpr::Atom {
                            atom: Atom::Unit,
                            span: stmt.span,
                        };
                        tail_span = stmt.span;
                    }
                }
                Stmt::IfThen(stmt) => {
                    self.if_then(&mut lets, stmt);
                    if index == last {
                        tail = CoreExpr::Atom {
                            atom: Atom::Unit,
                            span: stmt.span,
                        };
                        tail_span = stmt.span;
                    }
                }
                Stmt::Let(stmt) => {
                    let (value, span) = self.value(&mut lets, &stmt.value);
                    let written = self.records.annotations.get(&stmt.name.span).cloned();
                    let ty = match &written {
                        Some(declared) => self.declared(declared),
                        None => self.type_at(stmt.name.span),
                    };
                    let name = self.name(&stmt.name);
                    let binder = self.bind(name);
                    self.types.insert(binder.clone(), ty.clone());
                    bind_expr(&mut lets, binder, ty, value, span, written.is_some());
                }
                Stmt::Handled(handled) if self.marked(&handled.expr) => {
                    let atom = self.marked_call(&mut lets, handled);
                    if index == last {
                        tail = CoreExpr::Atom {
                            atom,
                            span: handled.expr.span,
                        };
                        tail_span = handled.expr.span;
                    }
                }
                Stmt::BlockExpr(expr) if !self.updates(expr).is_empty() => {
                    let span = block_expr_span(expr);
                    let atom = self.joined(&mut lets, expr, span);
                    if index == last {
                        tail = CoreExpr::Atom { atom, span };
                        tail_span = span;
                    }
                }
                Stmt::Handled(handled) if index == last => {
                    tail = self.handled(handled);
                    tail_span = handled.expr.span;
                }
                Stmt::BlockExpr(expr) if index == last => {
                    tail = self.block_expr(expr);
                    tail_span = block_expr_span(expr);
                }
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
        if let Some(wrap) = wrap
            && !wrapped
        {
            tail = self.wrap_tail(&mut lets, tail, wrap, tail_span);
        }
        self.scope.truncate(mark);
        self.current_join = join;
        close(lets, tail)
    }

    /// A statement's expression with its `with` bindings and handling clauses (`spec/09 §3`).
    fn handled(&mut self, handled: &Handled) -> CoreExpr {
        self.handled_in(handled, None).0
    }

    /// [`Self::handled`], and for a call with `edit` arguments, `tuple` holds the current
    /// values of its places and the types of its tuple: an `if error use` default keeps the
    /// places, and `if none` unwraps the last component (`spec/17 §4`, `§7` rule 1). Returns
    /// the expression and the types of its tuple.
    fn handled_in(
        &mut self,
        handled: &Handled,
        tuple: Option<(Vec<Atom>, Vec<Type>)>,
    ) -> (CoreExpr, Vec<Type>) {
        self.handled_body(handled, tuple, None)
    }

    /// [`Self::handled_in`], with `body` in place of the elaboration of `handled.expr` when it
    /// is given: the in-place update of a map, a set or a sequence under a deadline, whose
    /// tuple is `(new value, result)`.
    fn handled_body(
        &mut self,
        handled: &Handled,
        tuple: Option<(Vec<Atom>, Vec<Type>)>,
        body: Option<CoreExpr>,
    ) -> (CoreExpr, Vec<Type>) {
        let (places, mut components) = tuple.unwrap_or_default();
        let mut lets: Vec<Let> = Vec::new();
        let mut bindings = Vec::new();
        let span = handled.expr.span;
        // `e within d`: `d` is evaluated first (`spec/27 §5`).
        let deadline = handled
            .within
            .as_ref()
            .map(|deadline| (self.atom(&mut lets, deadline), deadline.span));
        for binding in &handled.with {
            let cap = self.name(&binding.cap).to_owned();
            let source = match &binding.value {
                // A narrowed rebinding runs as the binding it narrows: the narrowing is checked
                // statically, and the conditions of that binding still apply (ADR 0093 rules 4
                // and 13).
                CapBindingValue::Cap(other) | CapBindingValue::Narrowed { target: other, .. } => {
                    Source::Rebind(self.name(other).to_owned())
                }
                CapBindingValue::Handler(call) => {
                    let handler = match self.res(call.span) {
                        Some(Res::Handler { id }) => id,
                        _ => {
                            return (
                                CoreExpr::Atom {
                                    atom: Atom::Unit,
                                    span,
                                },
                                components,
                            );
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
                        span: call.span,
                    }
                }
            };
            bindings.push((cap, source));
        }
        let mut current = match body {
            Some(body) => body,
            None => self.expr(&handled.expr),
        };
        if let Some((atom, deadline_span)) = deadline {
            let passing = handled
                .handlings
                .iter()
                .any(|handling| matches!(handling.kind, HandlingKind::ErrorPass));
            current = self.within_core(
                current,
                atom,
                deadline_span,
                passing,
                span.to(deadline_span),
            );
        }
        let value_ty = self.type_at(handled.expr.span);
        for handling in &handled.handlings {
            current = match &handling.kind {
                HandlingKind::ErrorPass => current,
                HandlingKind::ErrorFail(variant) => {
                    let error = self.bind("error");
                    let outer = self.error.replace(error.clone());
                    let handler = self.fail(variant);
                    self.error = outer;
                    self.scope.pop();
                    CoreExpr::Handle {
                        body: Box::new(current),
                        error,
                        handler: Box::new(handler),
                    }
                }
                HandlingKind::ErrorUse(value) if !components.is_empty() => {
                    let error = self.bind("error");
                    let outer = self.error.replace(error.clone());
                    let mut inner = Vec::new();
                    let default = self.atom(&mut inner, value);
                    let mut atoms = places.clone();
                    atoms.push(default);
                    let t = self.temp();
                    inner.push(Let::inferred(
                        t.clone(),
                        tuple_type(components.clone()),
                        Bound::Rhs(Rhs::Tuple(atoms)),
                        value.span,
                    ));
                    let handler = close(
                        inner,
                        CoreExpr::Atom {
                            atom: Atom::Var(t),
                            span: value.span,
                        },
                    );
                    self.error = outer;
                    self.scope.pop();
                    CoreExpr::Handle {
                        body: Box::new(current),
                        error,
                        handler: Box::new(handler),
                    }
                }
                HandlingKind::NoneFail(variant) if !components.is_empty() => {
                    let otherwise = self.fail(variant);
                    self.unwrap_last(current, &mut components, otherwise, span)
                }
                HandlingKind::NoneUse(value) if !components.is_empty() => {
                    let otherwise = self.expr(value);
                    self.unwrap_last(current, &mut components, otherwise, span)
                }
                HandlingKind::ErrorUse(value) => {
                    let error = self.bind("error");
                    let outer = self.error.replace(error.clone());
                    let handler = self.expr(value);
                    self.error = outer;
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
        // S-41: the installations of a `with` under a raised program counter, which the checker
        // marked by their handler calls, run `quiet`, with the body they scope, which stands
        // under the same program counter (`spec/26 §6` rule 4).
        let quiet = handled.with.iter().any(|binding| match &binding.value {
            CapBindingValue::Handler(call) => self.findings.quiet.contains(&call.span),
            _ => false,
        });
        let hushed = handled.with.iter().any(|binding| match &binding.value {
            CapBindingValue::Handler(call) => self.findings.hushed.contains(&call.span),
            _ => false,
        });
        for (cap, source) in bindings.into_iter().rev() {
            current = CoreExpr::With {
                cap,
                source,
                body: Box::new(current),
            };
        }
        if quiet {
            current = CoreExpr::Quiet {
                body: Box::new(current),
                timing: !hushed,
                span,
            };
        }
        (close(lets, current), components)
    }

    /// `body` bounded by the deadline `deadline`, whose source is `deadline_span`, as the
    /// whole `e within d` at `span` (`spec/27 §5`, ADR 0117 rule 5). Its time-out is `TimedOut`
    /// of `Timed` with failures wrapped in `Failed`, or `Expired` of `Deadline`, as the checker
    /// typed it; under `if error pass` (`passing`), the function's own `TimedOut`, with failures
    /// passed as they are.
    fn within_core(
        &mut self,
        body: CoreExpr,
        deadline: Atom,
        deadline_span: Span,
        passing: bool,
        span: Span,
    ) -> CoreExpr {
        let variant = |fx: &mut Self, kind: usize, name: &str| -> Option<Ctor> {
            let index = fx
                .env
                .kinds
                .get(kind)?
                .variants
                .iter()
                .position(|v| v.name == name)?;
            Some(fx.ctor(kind, Some(index)))
        };
        let failure = self.records.withins.get(&deadline_span).cloned();
        let own = self.records.within_passes.get(&deadline_span).cloned();
        let shape = match (passing, own, failure) {
            (true, Some(Ty::Kind(own, _)), _) => {
                variant(self, own, "TimedOut").map(|timeout| (timeout, None))
            }
            (false, _, Some(Ty::Kind(kind, args))) if !args.is_empty() => {
                match (
                    variant(self, kind, "TimedOut"),
                    variant(self, kind, "Failed"),
                ) {
                    (Some(timeout), Some(wrap)) => Some((timeout, Some(wrap))),
                    _ => None,
                }
            }
            (false, _, Some(Ty::Kind(kind, _))) => {
                variant(self, kind, "Expired").map(|timeout| (timeout, None))
            }
            _ => None,
        };
        let Some((timeout, wrap)) = shape else {
            self.broken.set(true);
            return body;
        };
        let within = CoreExpr::Within {
            deadline,
            deadline_span,
            body: Box::new(body),
            timeout: Box::new(timeout),
            wrap: wrap.map(Box::new),
            span,
        };
        self.quiet_construct(deadline_span, within, span)
    }

    /// `if none` on the last component of a call's tuple: the places keep their new values,
    /// and the option is unwrapped.
    fn unwrap_last(
        &mut self,
        tuple: CoreExpr,
        components: &mut [Type],
        otherwise: CoreExpr,
        span: Span,
    ) -> CoreExpr {
        let mut lets = Vec::new();
        let t = self.temp();
        bind_expr(
            &mut lets,
            t.clone(),
            tuple_type(components.to_vec()),
            tuple,
            span,
            false,
        );
        let names: Vec<(Name, Type)> = components
            .iter()
            .map(|ty| (self.temp(), ty.clone()))
            .collect();
        lets.push(Let::destructure(names.clone(), Atom::Var(t), span));
        let Some(((option, option_ty), kept)) = names.split_last() else {
            self.broken.set(true);
            return close(lets, otherwise);
        };
        let inner = match option_ty {
            Type::Base {
                base: Base::Named { args, .. },
                ..
            } => args.first().cloned().unwrap_or_else(unknown),
            _ => unknown(),
        };
        let unwrapped = self.unwrap_option(
            CoreExpr::Atom {
                atom: Atom::Var(option.clone()),
                span,
            },
            option_ty.clone(),
            otherwise,
            span,
        );
        let v = self.temp();
        bind_expr(&mut lets, v.clone(), inner.clone(), unwrapped, span, false);
        if let Some(last) = components.last_mut() {
            *last = inner;
        }
        let mut atoms: Vec<Atom> = kept.iter().map(|(n, _)| Atom::Var(n.clone())).collect();
        atoms.push(Atom::Var(v));
        let r = self.temp();
        lets.push(Let::inferred(
            r.clone(),
            tuple_type(components.to_vec()),
            Bound::Rhs(Rhs::Tuple(atoms)),
            span,
        ));
        close(
            lets,
            CoreExpr::Atom {
                atom: Atom::Var(r),
                span,
            },
        )
    }

    // ----- the memory model (`spec/17 §4`) -----

    /// The Core result of a function: `(params..., result)` with its `edit` parameters, whose
    /// names are then kept for the body and the `ensures` clauses.
    fn returning(
        &mut self,
        defs: &[super::env::ParamDef<'a>],
        params: &[(Name, Type)],
        result: Type,
    ) -> Type {
        let edits: Vec<(String, Type)> = defs
            .iter()
            .zip(params)
            .filter(|(def, _)| def.convention == Some(Convention::Edit))
            .map(|(def, (_, ty))| (def.name.to_owned(), ty.clone()))
            .collect();
        if edits.is_empty() {
            self.returning = None;
            return result;
        }
        let mut components: Vec<Type> = edits.iter().map(|(_, ty)| ty.clone()).collect();
        components.push(result.clone());
        self.returning = Some((edits.into_iter().map(|(name, _)| name).collect(), result));
        tuple_type(components)
    }

    /// In an `ensures` clause, the component of the result that is the final value of the
    /// `edit` parameter `name`.
    fn ensures_edit(&self, name: &str) -> Option<usize> {
        if !self.in_ensures {
            return None;
        }
        let (edits, _) = self.returning.as_ref()?;
        edits.iter().position(|edit| edit == name)
    }

    /// The type of the current binder of the editable binding `name`.
    fn binding_type(&self, binder: &str, at: Span) -> Type {
        self.types
            .get(binder)
            .cloned()
            .unwrap_or_else(|| self.type_at(at))
    }

    /// The value of a `let` or of an assignment.
    fn value(&mut self, lets: &mut Vec<Let>, value: &LetValue) -> (CoreExpr, Span) {
        match value {
            LetValue::Handled(handled) if self.marked(&handled.expr) => {
                let span = handled.expr.span;
                let atom = self.marked_call(lets, handled);
                (CoreExpr::Atom { atom, span }, span)
            }
            LetValue::Handled(handled) => (self.handled(handled), handled.expr.span),
            LetValue::BlockExpr(expr) if !self.updates(expr).is_empty() => {
                let span = block_expr_span(expr);
                let atom = self.joined(lets, expr, span);
                (CoreExpr::Atom { atom, span }, span)
            }
            LetValue::BlockExpr(expr) => (self.block_expr(expr), block_expr_span(expr)),
        }
    }

    /// `p = value`: a new binder for the root of `p`, with the record rebuilt along a field
    /// path. A refined binding carries the obligation of an annotated `let` (`spec/17 §5`
    /// rule 2).
    fn assign(&mut self, lets: &mut Vec<Let>, place: &Expr, value: CoreExpr, span: Span) {
        match &place.kind {
            ExprKind::Paren(inner) => self.assign(lets, inner, value, span),
            ExprKind::Name(ident) => {
                let name = self.name(ident);
                let Some(old) = self.local(name) else {
                    self.broken.set(true);
                    return;
                };
                let ty = self.binding_type(&old, ident.span);
                let binder = self.bind(name);
                self.types.insert(binder.clone(), ty.clone());
                let written = has_refinements(&ty) || has_fn_type(&ty);
                bind_expr(lets, binder, ty, value, span, written);
            }
            ExprKind::Field { target, .. } => {
                let Some(Res::Field { kind, index }) = self.res(place.span) else {
                    self.broken.set(true);
                    return;
                };
                let v = self.temp();
                let field_ty = strip(self.field_fn_type(kind, index, target.span));
                bind_expr(lets, v.clone(), field_ty, value, span, false);
                let record = self.atom(lets, target);
                let count = self
                    .env
                    .kinds
                    .get(kind)
                    .and_then(|def| def.record.as_ref())
                    .map_or(0, Vec::len);
                let mut args = Vec::new();
                for i in 0..count {
                    if i == index {
                        args.push(Atom::Var(v.clone()));
                        continue;
                    }
                    let (_, field) = self.field(kind, i);
                    let ty = self.field_fn_type(kind, i, target.span);
                    let t = self.temp();
                    lets.push(Let::inferred(
                        t.clone(),
                        ty,
                        Bound::Rhs(Rhs::Field {
                            value: record.clone(),
                            field,
                            index: i,
                        }),
                        place.span,
                    ));
                    args.push(Atom::Var(t));
                }
                let ctor = self.ctor(kind, None);
                let rebuilt = Rhs::Construct {
                    ctor,
                    arg_spans: vec![span; args.len()],
                    args,
                };
                let r = self.temp();
                lets.push(Let::inferred(
                    r.clone(),
                    strip(self.type_at(target.span)),
                    Bound::Rhs(rebuilt),
                    span,
                ));
                self.assign(
                    lets,
                    target,
                    CoreExpr::Atom {
                        atom: Atom::Var(r),
                        span,
                    },
                    span,
                );
            }
            _ => self.broken.set(true),
        }
    }

    /// `if c` with a block and no `else`: `if c then block else unit`, with a join when the
    /// block updates bindings of the enclosing scope.
    fn if_then(&mut self, lets: &mut Vec<Let>, stmt: &IfThenStmt) {
        let names = self.block_updates(&stmt.block);
        let cond = self.atom(lets, &stmt.condition);
        if names.is_empty() {
            let then = self.block(&stmt.block);
            let expr = CoreExpr::If {
                cond,
                then: Box::new(then),
                otherwise: Box::new(CoreExpr::Atom {
                    atom: Atom::Unit,
                    span: stmt.span,
                }),
            };
            bind_expr(lets, "_".to_owned(), unit(), expr, stmt.span, false);
            return;
        }
        let join = Wrap::Join {
            names: names.clone(),
            ty: unit(),
        };
        self.tail_wrap = Some(join.clone());
        let then = self.block(&stmt.block);
        let mut inner = Vec::new();
        let unchanged = CoreExpr::Atom {
            atom: Atom::Unit,
            span: stmt.span,
        };
        let otherwise = self.wrap_tail(&mut inner, unchanged, join, stmt.span);
        let expr = CoreExpr::If {
            cond,
            then: Box::new(then),
            otherwise: Box::new(close(inner, otherwise)),
        };
        self.bind_join(lets, &names, unit(), expr, stmt.span);
    }

    /// An `if` or a `when` that updates bindings of the enclosing scope: each branch returns
    /// `(value, names...)`, and the construct binds them afresh (`spec/17 §4`, joins).
    fn joined(&mut self, lets: &mut Vec<Let>, expr: &BlockExpr, span: Span) -> Atom {
        let names = self.updates(expr);
        let ty = strip(self.type_at(span));
        let saved = self.current_join.replace(Wrap::Join {
            names: names.clone(),
            ty: ty.clone(),
        });
        let value = self.block_expr(expr);
        self.current_join = saved;
        self.bind_join(lets, &names, ty, value, span)
    }

    /// Binds the tuple of a join and destructures it, rebinding `names`; returns the value.
    fn bind_join(
        &mut self,
        lets: &mut Vec<Let>,
        names: &[String],
        ty: Type,
        value: CoreExpr,
        span: Span,
    ) -> Atom {
        let mut binders = Vec::new();
        let mut types = vec![ty.clone()];
        for name in names {
            let old = self.local(name).unwrap_or_default();
            let ty = self.binding_type(&old, span);
            types.push(ty.clone());
            binders.push((name.clone(), ty));
        }
        let t = self.temp();
        bind_expr(lets, t.clone(), tuple_type(types), value, span, false);
        let v = self.temp();
        let mut destructured = vec![(v.clone(), ty)];
        for (name, ty) in binders {
            let binder = self.bind(&name);
            self.types.insert(binder.clone(), ty.clone());
            destructured.push((binder, ty));
        }
        lets.push(Let::destructure(destructured, Atom::Var(t), span));
        Atom::Var(v)
    }

    /// The tail of a branch or of a function body, wrapped as `wrap` says.
    fn wrap_tail(
        &mut self,
        lets: &mut Vec<Let>,
        tail: CoreExpr,
        wrap: Wrap,
        span: Span,
    ) -> CoreExpr {
        let join = matches!(wrap, Wrap::Join { .. });
        let (names, ty) = match wrap {
            Wrap::State { names } => return self.wrap_state(lets, tail, &names, span),
            Wrap::Join { names, ty } => (names, ty),
            Wrap::Return => match self.returning.clone() {
                Some((names, ty)) => (names, ty),
                None => {
                    self.broken.set(true);
                    return tail;
                }
            },
        };
        let value_span = span;
        let value = match tail {
            CoreExpr::Atom { atom, .. } => atom,
            other => {
                let v = self.temp();
                bind_expr(lets, v.clone(), strip(ty.clone()), other, span, false);
                Atom::Var(v)
            }
        };
        let mut atoms = Vec::new();
        let mut types = Vec::new();
        for name in &names {
            let Some(binder) = self.local(name) else {
                self.broken.set(true);
                continue;
            };
            types.push(self.binding_type(&binder, span));
            atoms.push(Atom::Var(binder));
        }
        let (atoms, types) = if join {
            // `(value, names...)`
            let mut a = vec![value];
            a.extend(atoms);
            let mut t = vec![strip(ty)];
            t.extend(types);
            (a, t)
        } else {
            // `(params..., result)`
            atoms.push(value);
            types.push(ty);
            (atoms, types)
        };
        let t = self.temp();
        lets.push(Let::inferred(
            t.clone(),
            tuple_type(types),
            Bound::Rhs(Rhs::Tuple(atoms)),
            value_span,
        ));
        CoreExpr::Atom {
            atom: Atom::Var(t),
            span: value_span,
        }
    }

    /// A function's result written as a line: each branch of an `if` line returns
    /// `(params..., value)` at its own value, the return paths of `spec/12 §3`.
    fn return_line(&mut self, expr: &Expr) -> CoreExpr {
        match &expr.kind {
            ExprKind::Paren(inner) => self.return_line(inner),
            ExprKind::IfLine {
                condition,
                then,
                otherwise,
            } => {
                let mut lets = Vec::new();
                let cond = self.atom(&mut lets, condition);
                let then = self.return_line(then);
                let otherwise = self.return_line(otherwise);
                close(
                    lets,
                    CoreExpr::If {
                        cond,
                        then: Box::new(then),
                        otherwise: Box::new(otherwise),
                    },
                )
            }
            ExprKind::Fail(_) => self.expr(expr),
            _ => {
                let value = self.expr(expr);
                let mut lets = Vec::new();
                let tail = self.wrap_tail(&mut lets, value, Wrap::Return, expr.span);
                close(lets, tail)
            }
        }
    }

    /// The tail of a loop body: its value is discarded, and the body returns the current values
    /// of the bindings the loop updates.
    fn wrap_state(
        &mut self,
        lets: &mut Vec<Let>,
        tail: CoreExpr,
        names: &[String],
        span: Span,
    ) -> CoreExpr {
        if !matches!(tail, CoreExpr::Atom { .. }) {
            bind_expr(lets, "_".to_owned(), unit(), tail, span, false);
        }
        let mut atoms = Vec::new();
        let mut types = Vec::new();
        for name in names {
            let Some(binder) = self.local(name) else {
                self.broken.set(true);
                continue;
            };
            types.push(self.binding_type(&binder, span));
            atoms.push(Atom::Var(binder));
        }
        let t = self.temp();
        lets.push(Let::inferred(
            t.clone(),
            tuple_type(types),
            Bound::Rhs(Rhs::Tuple(atoms)),
            span,
        ));
        CoreExpr::Atom {
            atom: Atom::Var(t),
            span,
        }
    }

    /// `exit_k(b, ...)` with the current values of the innermost loop's state (`spec/26 §3`).
    fn exit(&mut self, tag: ExitTag, span: Span) -> CoreExpr {
        let Some(names) = self.loop_names.clone() else {
            // Outside a loop body: U2034, already reported.
            self.broken.set(true);
            return CoreExpr::Atom {
                atom: Atom::Unit,
                span,
            };
        };
        let mut state = Vec::new();
        for name in &names {
            match self.local(name) {
                Some(binder) => state.push(Atom::Var(binder)),
                None => self.broken.set(true),
            }
        }
        CoreExpr::Exit { tag, state, span }
    }

    /// `for x in source` (`spec/17 §4`): a Core loop over the bindings its body updates, whose
    /// final values are bound afresh after it (ADR 0056, amendment of 2026-10-02).
    fn for_loop(&mut self, lets: &mut Vec<Let>, stmt: &ForStmt) {
        let var_name = self.name(&stmt.var);
        let mut names = Vec::new();
        self.scan_block(&stmt.block, &[var_name.to_owned()], &mut names);
        names.retain(|name| self.local(name).is_some());
        let source = match &stmt.source {
            ForSource::List(list) => LoopSource::List(self.atom(lets, list)),
            ForSource::Range {
                low,
                high,
                exclusive,
            } => LoopSource::Range {
                low: self.atom(lets, low),
                high: self.atom(lets, high),
                inclusive: !exclusive,
            },
        };
        let mut init = Vec::new();
        let mut types = Vec::new();
        for name in &names {
            let binder = self.local(name).unwrap_or_default();
            types.push(self.binding_type(&binder, stmt.span));
            init.push(Atom::Var(binder));
        }
        let mark = self.scope.len();
        let var_ty = self.type_at(stmt.var.span);
        let var = (self.bind(var_name), var_ty);
        let mut state = Vec::new();
        for (name, ty) in names.iter().zip(&types) {
            let binder = self.bind(name);
            self.types.insert(binder.clone(), ty.clone());
            state.push((binder, ty.clone()));
        }
        self.tail_wrap = Some(Wrap::State {
            names: names.clone(),
        });
        let outer = self.loop_names.replace(names.clone());
        let body = self.block(&stmt.block);
        self.loop_names = outer;
        self.scope.truncate(mark);
        let looped = CoreExpr::Loop {
            source: Box::new(source),
            var: Box::new(var),
            state,
            init,
            body: Box::new(body),
            on_stop: None,
            span: stmt.span,
        };
        let t = self.temp();
        bind_expr(
            lets,
            t.clone(),
            tuple_type(types.clone()),
            looped,
            stmt.span,
            false,
        );
        if names.is_empty() {
            return;
        }
        let mut after = Vec::new();
        for (name, ty) in names.iter().zip(types) {
            let binder = self.bind(name);
            self.types.insert(binder.clone(), ty.clone());
            after.push((binder, ty));
        }
        lets.push(Let::destructure(after, Atom::Var(t), stmt.span));
    }

    /// `for until c decreases m` (`spec/26 §4`): a Core loop from `until(c)` over the bindings
    /// its body updates, `c` evaluated before each iteration with the state bound, and `m` a
    /// clause over the state for the verifier only (ADR 0116 rules 7 and 20).
    fn until_loop(&mut self, lets: &mut Vec<Let>, stmt: &UntilStmt) {
        let mut names = Vec::new();
        self.scan_block(&stmt.block, &[], &mut names);
        names.retain(|name| self.local(name).is_some());
        let mut init = Vec::new();
        let mut types = Vec::new();
        for name in &names {
            let binder = self.local(name).unwrap_or_default();
            types.push(self.binding_type(&binder, stmt.span));
            init.push(Atom::Var(binder));
        }
        let mark = self.scope.len();
        let var = (self.temp(), unit());
        let mut state = Vec::new();
        for (name, ty) in names.iter().zip(&types) {
            let binder = self.bind(name);
            self.types.insert(binder.clone(), ty.clone());
            state.push((binder, ty.clone()));
        }
        // The condition and the measure read the state of the iteration about to start.
        let mut cond_lets = Vec::new();
        let cond_atom = self.atom(&mut cond_lets, &stmt.condition);
        let cond = close(
            cond_lets,
            CoreExpr::Atom {
                atom: cond_atom,
                span: stmt.condition.span,
            },
        );
        let holds = self
            .findings
            .refinable_conditions
            .contains(&stmt.condition.span)
            .then(|| Box::new(self.clause(&stmt.condition)));
        let measure = Box::new(self.clause(&stmt.measure));
        let source = LoopSource::Until {
            cond: Box::new(cond),
            holds,
            measure,
            decreases: stmt.decreases,
        };
        self.tail_wrap = Some(Wrap::State {
            names: names.clone(),
        });
        let outer = self.loop_names.replace(names.clone());
        let body = self.block(&stmt.block);
        self.loop_names = outer;
        self.scope.truncate(mark);
        let looped = CoreExpr::Loop {
            source: Box::new(source),
            var: Box::new(var),
            state,
            init,
            body: Box::new(body),
            on_stop: None,
            span: stmt.span,
        };
        let t = self.temp();
        bind_expr(
            lets,
            t.clone(),
            tuple_type(types.clone()),
            looped,
            stmt.span,
            false,
        );
        if names.is_empty() {
            return;
        }
        let mut after = Vec::new();
        for (name, ty) in names.iter().zip(types) {
            let binder = self.bind(name);
            self.types.insert(binder.clone(), ty.clone());
            after.push((binder, ty));
        }
        lets.push(Let::destructure(after, Atom::Var(t), stmt.span));
    }

    /// `for ever` with its `on stop` (`spec/26 §5`): a Core loop from `ever` over the bindings
    /// its body and its `on stop` update, whose `on stop` block binds the state afresh and
    /// returns it (ADR 0116 rules 10 and 20).
    fn ever_loop(&mut self, lets: &mut Vec<Let>, stmt: &EverStmt) {
        // `within d`: evaluated once, before the first iteration (`spec/26 §7`).
        let within = stmt
            .within
            .as_ref()
            .map(|deadline| (self.atom(lets, deadline), deadline.span));
        let mut names = Vec::new();
        self.scan_block(&stmt.block, &[], &mut names);
        if let Some(on_stop) = &stmt.on_stop {
            self.scan_block(&on_stop.block, &[], &mut names);
        }
        names.retain(|name| self.local(name).is_some());
        let mut init = Vec::new();
        let mut types = Vec::new();
        for name in &names {
            let binder = self.local(name).unwrap_or_default();
            types.push(self.binding_type(&binder, stmt.span));
            init.push(Atom::Var(binder));
        }
        let mark = self.scope.len();
        let var = (self.temp(), unit());
        let mut state = Vec::new();
        for (name, ty) in names.iter().zip(&types) {
            let binder = self.bind(name);
            self.types.insert(binder.clone(), ty.clone());
            state.push((binder, ty.clone()));
        }
        self.tail_wrap = Some(Wrap::State {
            names: names.clone(),
        });
        let outer = self.loop_names.replace(names.clone());
        let body = self.block(&stmt.block);
        self.loop_names = outer;
        self.scope.truncate(mark);
        let on_stop = stmt.on_stop.as_ref().map(|on_stop| {
            let mut binders = Vec::new();
            for (name, ty) in names.iter().zip(&types) {
                let binder = self.bind(name);
                self.types.insert(binder.clone(), ty.clone());
                binders.push((binder, ty.clone()));
            }
            self.tail_wrap = Some(Wrap::State {
                names: names.clone(),
            });
            // No `stop` or `skip` of this loop in its `on stop` (U2034).
            let outer = self.loop_names.take();
            let body = self.block(&on_stop.block);
            self.loop_names = outer;
            self.scope.truncate(mark);
            Box::new(una_core::OnStop {
                state: binders,
                body,
            })
        });
        let looped = CoreExpr::Loop {
            source: Box::new(LoopSource::Ever { within }),
            var: Box::new(var),
            state,
            init,
            body: Box::new(body),
            on_stop,
            span: stmt.span,
        };
        // S-39: under a raised program counter, the loop is `quiet`.
        let looped = if self.findings.quiet.contains(&stmt.span) {
            CoreExpr::Quiet {
                body: Box::new(looped),
                timing: !self.findings.hushed.contains(&stmt.span),
                span: stmt.span,
            }
        } else {
            looped
        };
        let t = self.temp();
        bind_expr(
            lets,
            t.clone(),
            tuple_type(types.clone()),
            looped,
            stmt.span,
            false,
        );
        if names.is_empty() {
            return;
        }
        let mut after = Vec::new();
        for (name, ty) in names.iter().zip(types) {
            let binder = self.bind(name);
            self.types.insert(binder.clone(), ty.clone());
            after.push((binder, ty));
        }
        lets.push(Let::destructure(after, Atom::Var(t), stmt.span));
    }

    /// `tasks [at most b at once]` with its block: `let (x₁, ..., xₙ) = tasks(b) e`, where `e`
    /// is the block, which returns the new values of the outer bindings it updates, as the
    /// block of an `if` without `else` does (`spec/27 §7`, `spec/17 §4`). `stop` and `skip` in
    /// it stay those of the enclosing loop, whose `exit`s cross the statement.
    fn tasks(&mut self, lets: &mut Vec<Let>, stmt: &TasksStmt) {
        let bound = stmt
            .bound
            .as_ref()
            .map(|bound| (self.atom(lets, bound), bound.span));
        let names = self.block_updates(&stmt.block);
        if names.is_empty() {
            let body = self.block(&stmt.block);
            let expr = CoreExpr::Tasks {
                bound,
                body: Box::new(body),
                span: stmt.span,
            };
            let expr = self.quiet_construct(stmt.span, expr, stmt.span);
            bind_expr(lets, "_".to_owned(), unit(), expr, stmt.span, false);
            return;
        }
        self.tail_wrap = Some(Wrap::Join {
            names: names.clone(),
            ty: unit(),
        });
        let body = self.block(&stmt.block);
        let expr = CoreExpr::Tasks {
            bound,
            body: Box::new(body),
            span: stmt.span,
        };
        let expr = self.quiet_construct(stmt.span, expr, stmt.span);
        self.bind_join(lets, &names, unit(), expr, stmt.span);
    }

    /// `start e`: `start(e)`, whose task is `e` with its deadline and handling clauses, closed
    /// over the bindings before it, its tail never a return (`spec/27 §7`).
    fn start(&mut self, lets: &mut Vec<Let>, stmt: &StartStmt) {
        // A started task is not a loop body for `skip` and `stop` (U2034).
        let outer = self.loop_names.take();
        let wrap = self.tail_wrap.take();
        let join = self.current_join.take();
        let mark = self.scope.len();
        // The captures that can hold a reply right, by their binders before the task's own
        // names (ADR 0123 rule 4.2), in the order of their names.
        let rights: Vec<Name> = self
            .findings
            .start_rights
            .get(&stmt.keyword)
            .map(|names| {
                let binders: std::collections::BTreeSet<Name> = names
                    .iter()
                    .filter_map(|name| self.scope.find(name))
                    .collect();
                binders.into_iter().collect()
            })
            .unwrap_or_default();
        let handled = &stmt.handled;
        let mut inner = Vec::new();
        let value = if self.marked(&handled.expr) {
            let atom = self.marked_call(&mut inner, handled);
            CoreExpr::Atom {
                atom,
                span: handled.expr.span,
            }
        } else {
            self.handled(handled)
        };
        // The task's own names end with it.
        self.scope.truncate(mark);
        self.tail_wrap = wrap;
        self.current_join = join;
        self.loop_names = outer;
        let expr = CoreExpr::Start {
            task: Box::new(close(inner, value)),
            rights,
            span: stmt.span,
        };
        // S-40 (#1512): a `start` under a raised program counter is `quiet`.
        let expr = self.quiet_construct(stmt.span, expr, stmt.span);
        bind_expr(lets, "_".to_owned(), unit(), expr, stmt.span, false);
    }

    /// `together` with branches: `let (x₁, ..., xₙ) = together(e₁ ∥ ... ∥ eₙ)`, where each task
    /// `eᵢ` binds its branch's value as its `let` would, the refinement of a written type an
    /// obligation there, and returns it (`spec/19 §5`).
    fn together(&mut self, lets: &mut Vec<Let>, stmt: &TogetherStmt) {
        // A task is not a loop body for `skip` and `stop` (U2034).
        let outer = self.loop_names.take();
        self.together_tasks(lets, stmt);
        self.loop_names = outer;
    }

    fn together_tasks(&mut self, lets: &mut Vec<Let>, stmt: &TogetherStmt) {
        let mut tasks = Vec::new();
        let mut types = Vec::new();
        let mut names = Vec::new();
        for branch in &stmt.body {
            let Stmt::Let(branch) = branch else {
                self.broken.set(true);
                return;
            };
            let mark = self.scope.len();
            let mut inner = Vec::new();
            let (value, span) = self.value(&mut inner, &branch.value);
            let written = self.records.annotations.get(&branch.name.span).cloned();
            let ty = match &written {
                Some(declared) => self.declared(declared),
                None => self.type_at(branch.name.span),
            };
            let name = self.name(&branch.name);
            let binder = self.bind(name);
            self.types.insert(binder.clone(), ty.clone());
            bind_expr(
                &mut inner,
                binder.clone(),
                ty.clone(),
                value,
                span,
                written.is_some(),
            );
            // The task's own names end with it: its siblings and the code after the block
            // see only its result.
            self.scope.truncate(mark);
            tasks.push(close(
                inner,
                CoreExpr::Atom {
                    atom: Atom::Var(binder),
                    span,
                },
            ));
            types.push(ty);
            names.push(name);
        }
        let t = self.temp();
        bind_expr(
            lets,
            t.clone(),
            tuple_type(types.clone()),
            self.quiet_construct(
                stmt.span,
                CoreExpr::Together {
                    tasks,
                    span: stmt.span,
                },
                stmt.span,
            ),
            stmt.span,
            false,
        );
        let mut after = Vec::new();
        for (name, ty) in names.into_iter().zip(types) {
            let binder = self.bind(name);
            self.types.insert(binder.clone(), ty.clone());
            after.push((binder, ty));
        }
        lets.push(Let::destructure(after, Atom::Var(t), stmt.span));
    }

    /// `together for x in s` with its block: `together_for x in s. e`, whose task is the
    /// block, with no wrapping of its tail (`spec/19 §5`).
    fn together_for(&mut self, together: &TogetherFor) -> CoreExpr {
        let outer = self.loop_names.take();
        let expr = self.together_for_tasks(together);
        self.loop_names = outer;
        expr
    }

    fn together_for_tasks(&mut self, together: &TogetherFor) -> CoreExpr {
        let mut lets = Vec::new();
        let source = match &together.source {
            ForSource::List(list) => LoopSource::List(self.atom(&mut lets, list)),
            ForSource::Range {
                low,
                high,
                exclusive,
            } => LoopSource::Range {
                low: self.atom(&mut lets, low),
                high: self.atom(&mut lets, high),
                inclusive: !exclusive,
            },
        };
        let wrap = self.tail_wrap.take();
        let join = self.current_join.take();
        let mark = self.scope.len();
        let var_ty = self.type_at(together.var.span);
        let var = (self.bind(self.name(&together.var)), var_ty);
        let task = self.block(&together.block);
        self.scope.truncate(mark);
        self.tail_wrap = wrap;
        self.current_join = join;
        // Bound to a temporary, so that the verifier gives the list its length wherever it
        // stands (`spec/19 §4` rule 2).
        let t = self.temp();
        let ty = self.type_at(together.span);
        bind_expr(
            &mut lets,
            t.clone(),
            ty,
            self.quiet_construct(
                together.span,
                CoreExpr::TogetherFor {
                    source: Box::new(source),
                    var: Box::new(var),
                    task: Box::new(task),
                    span: together.span,
                },
                together.span,
            ),
            together.span,
            false,
        );
        close(
            lets,
            CoreExpr::Atom {
                atom: Atom::Var(t),
                span: together.span,
            },
        )
    }

    /// `first of` with its branches: `first_of(e₁ | ... | eₙ)`, whose tasks are the branches'
    /// expressions with their `with` bindings and handling clauses, each closed over the
    /// bindings before the block, bound to a temporary (`spec/27 §6`, ADR 0117 rule 6).
    fn first_of(&mut self, first: &FirstOf) -> CoreExpr {
        // A branch is not a loop body for `skip` and `stop` (U2034), and its tail is no return.
        let outer = self.loop_names.take();
        let wrap = self.tail_wrap.take();
        let join = self.current_join.take();
        let mut tasks = Vec::new();
        for branch in &first.body {
            let Stmt::Handled(handled) = branch else {
                self.broken.set(true);
                continue;
            };
            let mark = self.scope.len();
            let mut inner = Vec::new();
            let value = if self.marked(&handled.expr) {
                let atom = self.marked_call(&mut inner, handled);
                CoreExpr::Atom {
                    atom,
                    span: handled.expr.span,
                }
            } else {
                self.handled(handled)
            };
            // The branch's own names end with it.
            self.scope.truncate(mark);
            tasks.push(close(inner, value));
        }
        self.tail_wrap = wrap;
        self.current_join = join;
        self.loop_names = outer;
        let t = self.temp();
        let ty = self.type_at(first.span);
        let mut lets = Vec::new();
        bind_expr(
            &mut lets,
            t.clone(),
            ty,
            self.quiet_construct(
                first.span,
                CoreExpr::FirstOf {
                    tasks,
                    span: first.span,
                },
                first.span,
            ),
            first.span,
            false,
        );
        close(
            lets,
            CoreExpr::Atom {
                atom: Atom::Var(t),
                span: first.span,
            },
        )
    }

    /// Whether `expr` is a call with an `edit` argument or receiver.
    fn marked(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Call { args, .. } => args.iter().any(|a| a.marker == Some(Convention::Edit)),
            ExprKind::MethodCall { receiver, args, .. } => {
                *receiver == Some(Convention::Edit)
                    || args.iter().any(|a| a.marker == Some(Convention::Edit))
            }
            _ => false,
        }
    }

    /// The `edit` places of the call `expr`, in parameter order.
    fn edit_places<'x>(&self, expr: &'x Expr) -> Vec<&'x Expr> {
        let mut written: Vec<(Option<Convention>, &Expr)> = Vec::new();
        let args = match &expr.kind {
            ExprKind::Call { args, .. } => args,
            ExprKind::MethodCall {
                receiver,
                target,
                args,
                ..
            } => {
                written.push((*receiver, target));
                args
            }
            _ => return Vec::new(),
        };
        written.extend(args.iter().map(|a| (a.marker, &a.value)));
        let order = self.order(expr.span);
        let mut places: Vec<(usize, &Expr)> = written
            .into_iter()
            .zip(order)
            .filter_map(|((marker, value), slot)| {
                (marker == Some(Convention::Edit)).then_some((slot?, value))
            })
            .collect();
        places.sort_by_key(|(slot, _)| *slot);
        places.into_iter().map(|(_, value)| value).collect()
    }

    /// A statement or `let` whose expression is a call with `edit` arguments: the call
    /// returns `(places..., result)`, each place takes its new value, and the result is
    /// returned (`spec/17 §4`). The places change only when the call returns (`§7` rule 1).
    fn marked_call(&mut self, lets: &mut Vec<Let>, handled: &Handled) -> Atom {
        let expr = &handled.expr;
        let span = expr.span;
        // `edit m.set(key: k, value: v)` is `m = with(m, k, v)`, and `edit m.remove(key: k)`
        // is `m = without(m, k)` (`spec/21 §2` rule 1); `edit s.add(item: x)` is
        // `s = with(s, x)`, and `edit s.remove(item: x)` is `s = without(s, x)` (`spec/28 §6`), as
        // they are on a bit set (`spec/28 §9`).
        let update = match self.res(span) {
            Some(Res::MapUpdate(op)) => Some(Prim::Map(op)),
            Some(Res::SetUpdate(op)) => Some(Prim::Set(op)),
            // The same for a hash map and a hash set (`spec/28 §7`).
            Some(Res::HashMapUpdate(op)) => Some(Prim::HashMap(op)),
            Some(Res::HashSetUpdate(op)) => Some(Prim::HashSet(op)),
            Some(Res::BitSetUpdate(op)) => Some(Prim::BitSet(op)),
            _ => None,
        };
        if handled.within.is_some()
            && let Some(atom) = self.update_within(lets, handled)
        {
            return atom;
        }
        if let Some(prim) = update {
            let ExprKind::MethodCall { target, args, .. } = &expr.kind else {
                self.broken.set(true);
                return Atom::Unit;
            };
            let mut arg_spans = vec![target.span];
            let mut values = vec![self.atom(lets, target)];
            for arg in args {
                arg_spans.push(arg.value.span);
                values.push(self.atom(lets, &arg.value));
            }
            let t = self.temp();
            let ty = self.type_at(target.span);
            lets.push(Let::inferred(
                t.clone(),
                ty,
                Bound::Rhs(Rhs::Prim {
                    prim,
                    args: values,
                    arg_spans,
                    span,
                }),
                span,
            ));
            let value = CoreExpr::Atom {
                atom: Atom::Var(t),
                span: target.span,
            };
            self.assign(lets, target, value, span);
            return Atom::Unit;
        }
        if let Some(Res::SeqUpdate(op)) = self.res(span) {
            return self.removing_update(lets, expr, Prim::Seq(op), op.removed().map(Prim::Seq));
        }
        if let Some(Res::PriorityUpdate { op, read }) = self.res(span) {
            return self.removing_update(lets, expr, Prim::Priority(op), read.map(Prim::Priority));
        }
        let Some(Res::Function { id, type_args }) = self.res(span) else {
            self.broken.set(true);
            return Atom::Unit;
        };
        let Some(def) = self.env.functions.get(id) else {
            self.broken.set(true);
            return Atom::Unit;
        };
        // ADR 0120: a function that computes new elements keeps no refinement of its type
        // arguments in the new values of its `edit` places either (`super::dense::SYNTHESIZING`),
        // so a place whose declared type claims one gets an obligation, as its call's result
        // does (review of #1048, R2).
        let type_args: Vec<Ty> =
            if super::dense::synthesizes(self.env.prelude, def.module, def.name) {
                type_args.iter().map(Ty::without_refinements).collect()
            } else {
                type_args
            };
        let mut components: Vec<Type> = Vec::new();
        for param in &def.params {
            if param.convention == Some(Convention::Edit) {
                let declared = Declared {
                    base: param.ty.base.substitute(&type_args),
                    refinements: param.ty.refinements.clone(),
                };
                components.push(self.declared(&declared));
            }
        }
        components.push(self.declared(&Declared {
            base: def.result.base.substitute(&type_args),
            refinements: def.result.refinements.clone(),
        }));
        let places = self.edit_places(expr);
        if places.len() + 1 != components.len() {
            self.broken.set(true);
            return Atom::Unit;
        }
        let current: Vec<Atom> = places.iter().map(|place| self.atom(lets, place)).collect();
        self.call_tuple = Some((span, tuple_type(components.clone())));
        let (value, components) = self.handled_in(handled, Some((current, components)));
        self.call_tuple = None;
        let t = self.temp();
        bind_expr(
            lets,
            t.clone(),
            tuple_type(components.clone()),
            value,
            span,
            false,
        );
        let binders: Vec<(Name, Type)> = components
            .iter()
            .map(|ty| (self.temp(), ty.clone()))
            .collect();
        lets.push(Let::destructure(binders.clone(), Atom::Var(t), span));
        for (place, (binder, _)) in places.iter().zip(&binders) {
            let value = CoreExpr::Atom {
                atom: Atom::Var(binder.clone()),
                span: place.span,
            };
            self.assign(lets, place, value, place.span);
        }
        binders
            .last()
            .map_or(Atom::Unit, |(name, _)| Atom::Var(name.clone()))
    }

    /// The in-place update of a map, a set or a sequence bounded by `within` (`spec/27 §5`):
    /// the update, and the read of a removal, run inside the `within`, as `(new value, result)`;
    /// the target takes the new value only when the `within` succeeds, and keeps its value from
    /// before otherwise, as a call's `edit` places do (`spec/17 §7` rule 1). `None` when
    /// `handled` is no such update.
    fn update_within(&mut self, lets: &mut Vec<Let>, handled: &Handled) -> Option<Atom> {
        let expr = &handled.expr;
        let span = expr.span;
        let (prim, read) = match self.res(span) {
            Some(Res::MapUpdate(op)) => (Prim::Map(op), None),
            Some(Res::SetUpdate(op)) => (Prim::Set(op), None),
            Some(Res::SeqUpdate(op)) => (Prim::Seq(op), op.removed().map(Prim::Seq)),
            _ => return None,
        };
        let ExprKind::MethodCall { target, args, .. } = &expr.kind else {
            self.broken.set(true);
            return Some(Atom::Unit);
        };
        // Only the place's current value is read before the `within`; the arguments are
        // computed inside it, so its deadline bounds them, after `d` (`spec/27 §5` rules 2
        // and 5), as for a call with `edit` arguments.
        let mut arg_spans = vec![target.span];
        let old = self.atom(lets, target);
        let mut values = vec![old.clone()];
        let mut inner = Vec::new();
        for arg in args {
            arg_spans.push(arg.value.span);
            values.push(self.atom(&mut inner, &arg.value));
        }
        let target_ty = self.type_at(target.span);
        let result_ty = self.type_at(span);
        let result = match read {
            Some(read) => {
                let r = self.temp();
                inner.push(Let::inferred(
                    r.clone(),
                    result_ty.clone(),
                    Bound::Rhs(Rhs::Prim {
                        prim: read,
                        args: values.clone(),
                        arg_spans: arg_spans.clone(),
                        span,
                    }),
                    span,
                ));
                Atom::Var(r)
            }
            None => Atom::Unit,
        };
        let t = self.temp();
        inner.push(Let::inferred(
            t.clone(),
            target_ty.clone(),
            Bound::Rhs(Rhs::Prim {
                prim,
                args: values,
                arg_spans,
                span,
            }),
            span,
        ));
        let components = vec![target_ty, result_ty];
        let pair = self.temp();
        inner.push(Let::inferred(
            pair.clone(),
            tuple_type(components.clone()),
            Bound::Rhs(Rhs::Tuple(vec![Atom::Var(t), result])),
            span,
        ));
        let body = close(
            inner,
            CoreExpr::Atom {
                atom: Atom::Var(pair),
                span,
            },
        );
        let (value, components) =
            self.handled_body(handled, Some((vec![old], components)), Some(body));
        let whole = self.temp();
        bind_expr(
            lets,
            whole.clone(),
            tuple_type(components.clone()),
            value,
            span,
            false,
        );
        let binders: Vec<(Name, Type)> = components
            .iter()
            .map(|ty| (self.temp(), ty.clone()))
            .collect();
        lets.push(Let::destructure(binders.clone(), Atom::Var(whole), span));
        let (Some((updated, _)), Some((result, _))) = (binders.first(), binders.get(1)) else {
            self.broken.set(true);
            return Some(Atom::Unit);
        };
        let value = CoreExpr::Atom {
            atom: Atom::Var(updated.clone()),
            span: target.span,
        };
        self.assign(lets, target, value, span);
        Some(Atom::Var(result.clone()))
    }

    /// `edit q.push(item: x)`, `edit q.pop()` and the other updates of a `Stack` or a `Queue`
    /// (`spec/28 §5`) and of a `PriorityQueue` (`spec/28 §8`): `q = update(q, x)`, as a map's
    /// `set` is `m = with(m, k, v)`. A removal first reads what it removes, by `read`
    /// (`PeekFront` or `PeekBack` of a sequence, `Peek` or `PeekRanked` of a priority queue),
    /// which is the call's value; a push's value is `unit`. The read holds no reference to the
    /// collection, so the removal still updates it in place when nothing else holds it. O(1)
    /// besides the arguments.
    fn removing_update(
        &mut self,
        lets: &mut Vec<Let>,
        expr: &Expr,
        update: Prim,
        read: Option<Prim>,
    ) -> Atom {
        let span = expr.span;
        let ExprKind::MethodCall { target, args, .. } = &expr.kind else {
            self.broken.set(true);
            return Atom::Unit;
        };
        let mut arg_spans = vec![target.span];
        let mut values = vec![self.atom(lets, target)];
        for arg in args {
            arg_spans.push(arg.value.span);
            values.push(self.atom(lets, &arg.value));
        }
        let result = match read {
            Some(read) => {
                let r = self.temp();
                let ty = self.type_at(span);
                lets.push(Let::inferred(
                    r.clone(),
                    ty,
                    Bound::Rhs(Rhs::Prim {
                        prim: read,
                        args: values.clone(),
                        arg_spans: arg_spans.clone(),
                        span,
                    }),
                    span,
                ));
                Atom::Var(r)
            }
            None => Atom::Unit,
        };
        let t = self.temp();
        let ty = self.type_at(target.span);
        lets.push(Let::inferred(
            t.clone(),
            ty,
            Bound::Rhs(Rhs::Prim {
                prim: update,
                args: values,
                arg_spans,
                span,
            }),
            span,
        ));
        let value = CoreExpr::Atom {
            atom: Atom::Var(t),
            span: target.span,
        };
        self.assign(lets, target, value, span);
        result
    }

    /// The bindings of the enclosing scope that an `if` or a `when` updates, in order of
    /// first update.
    fn updates(&self, expr: &BlockExpr) -> Vec<String> {
        let mut found = Vec::new();
        self.scan_block_expr(expr, &[], &mut found);
        found.retain(|name| self.local(name).is_some());
        found
    }

    /// [`Self::updates`] of a block.
    fn block_updates(&self, block: &Block) -> Vec<String> {
        let mut found = Vec::new();
        self.scan_block(block, &[], &mut found);
        found.retain(|name| self.local(name).is_some());
        found
    }

    fn scan_block_expr(&self, expr: &BlockExpr, declared: &[String], found: &mut Vec<String>) {
        match expr {
            BlockExpr::If(block) => self.scan_if(block, declared, found),
            BlockExpr::TogetherFor(together) => {
                let mut inner = declared.to_vec();
                inner.push(self.name(&together.var).to_owned());
                self.scan_block(&together.block, &inner, found);
            }
            // A branch updates no binding declared outside it (U2023).
            BlockExpr::FirstOf(_) => {}
            BlockExpr::When(when) => {
                for arm in &when.arms {
                    if let Body::Block(block) = &arm.body {
                        self.scan_block(block, declared, found);
                    }
                }
            }
        }
    }

    fn scan_if(&self, block: &IfBlock, declared: &[String], found: &mut Vec<String>) {
        self.scan_block(&block.then_block, declared, found);
        match block.otherwise.as_ref() {
            ElseBranch::Block(otherwise) => self.scan_block(otherwise, declared, found),
            ElseBranch::If(next) => self.scan_if(next, declared, found),
        }
    }

    fn scan_block(&self, block: &Block, declared: &[String], found: &mut Vec<String>) {
        let mut local = declared.to_vec();
        for stmt in &block.stmts {
            match stmt {
                Stmt::Let(stmt) => {
                    self.scan_value(&stmt.value, &local, found);
                    local.push(self.name(&stmt.name).to_owned());
                }
                Stmt::Assign(stmt) => {
                    self.scan_value(&stmt.value, &local, found);
                    if let Some((root, _)) = super::memory::place_root(&stmt.place) {
                        note_update(self.name(root), &local, found);
                    }
                }
                Stmt::IfThen(stmt) => self.scan_block(&stmt.block, &local, found),
                Stmt::For(stmt) => {
                    let mut inner = local.clone();
                    inner.push(self.name(&stmt.var).to_owned());
                    self.scan_block(&stmt.block, &inner, found);
                }
                Stmt::Together(stmt) => {
                    let before = local.clone();
                    for branch in &stmt.body {
                        if let Stmt::Let(branch) = branch {
                            self.scan_value(&branch.value, &before, found);
                            local.push(self.name(&branch.name).to_owned());
                        }
                    }
                }
                Stmt::Handled(handled) => {
                    for place in self.edit_places_written(&handled.expr) {
                        if let Some((root, _)) = super::memory::place_root(place) {
                            note_update(self.name(root), &local, found);
                        }
                    }
                }
                Stmt::BlockExpr(expr) => self.scan_block_expr(expr, &local, found),
                Stmt::Stop(_) | Stmt::Skip(_) => {}
                Stmt::Until(stmt) => self.scan_block(&stmt.block, &local, found),
                Stmt::Ever(stmt) => {
                    self.scan_block(&stmt.block, &local, found);
                    if let Some(on_stop) = &stmt.on_stop {
                        self.scan_block(&on_stop.block, &local, found);
                    }
                }
                Stmt::OnStop(on_stop) => self.scan_block(&on_stop.block, &local, found),
                Stmt::Tasks(stmt) => self.scan_block(&stmt.block, &local, found),
                // A started task updates no binding declared outside it (U2023).
                Stmt::Start(_) => {}
            }
        }
    }

    fn scan_value(&self, value: &LetValue, declared: &[String], found: &mut Vec<String>) {
        match value {
            LetValue::Handled(handled) => {
                for place in self.edit_places_written(&handled.expr) {
                    if let Some((root, _)) = super::memory::place_root(place) {
                        note_update(self.name(root), declared, found);
                    }
                }
            }
            LetValue::BlockExpr(expr) => self.scan_block_expr(expr, declared, found),
        }
    }

    /// The `edit` places of a call as written, without its resolution.
    fn edit_places_written<'x>(&self, expr: &'x Expr) -> Vec<&'x Expr> {
        let mut places = Vec::new();
        let args = match &expr.kind {
            ExprKind::Call { args, .. } => args,
            ExprKind::MethodCall {
                receiver,
                target,
                args,
                ..
            } => {
                if *receiver == Some(Convention::Edit) {
                    places.push(target.as_ref());
                }
                args
            }
            _ => return places,
        };
        places.extend(
            args.iter()
                .filter(|a| a.marker == Some(Convention::Edit))
                .map(|a| &a.value),
        );
        places
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
            self.broken.set(true);
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
            BlockExpr::TogetherFor(together) => self.together_for(together),
            BlockExpr::FirstOf(first) => self.first_of(first),
        }
    }

    fn if_block(&mut self, block: &IfBlock) -> CoreExpr {
        let mut lets = Vec::new();
        let cond = self.atom(&mut lets, &block.condition);
        self.tail_wrap = self.current_join.clone();
        let then = self.block(&block.then_block);
        let otherwise = match block.otherwise.as_ref() {
            ElseBranch::Block(otherwise) => {
                self.tail_wrap = self.current_join.clone();
                self.block(otherwise)
            }
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
                            // An arm left out would send its values to another arm.
                            _ => {
                                self.broken.set(true);
                                continue;
                            }
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
                        self.tail_wrap = self.current_join.clone();
                        self.tail_wrap = self.current_join.clone();
                        let body = self.body(&arm.body);
                        self.scope.truncate(mark);
                        arms.push(CoreArm {
                            ctor,
                            binds: slots,
                            body,
                        });
                    }
                    PatternKind::Else => {
                        self.tail_wrap = self.current_join.clone();
                        otherwise = Some(Box::new(self.body(&arm.body)));
                    }
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
            self.tail_wrap = self.current_join.clone();
            let body = self.body(&arm.body);
            let test = match &arm.pattern.kind {
                PatternKind::Literal(literal) => {
                    let mut test_lets = Vec::new();
                    let value =
                        self.pattern_literal_atom(&mut test_lets, literal, arm.pattern.span);
                    let t = self.temp();
                    // The test depends on the scrutinee, so it carries its label (S-27).
                    let label = self.computed_label(when.scrutinee.span);
                    test_lets.push(Let::inferred(
                        t.clone(),
                        with_label(bool_ty(), &label),
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

    /// A pattern's literal, which is a constant (`spec/07 §7` rule 3): the checks reject a text
    /// literal with interpolation in a pattern (U4008, S-28). One that reaches here anyway is
    /// recorded as lost, for U8001, as a lost component of a checked type is, and the
    /// declaration is broken: never exported as checked Core, never silently built.
    fn pattern_literal_atom(&mut self, lets: &mut Vec<Let>, literal: &Literal, span: Span) -> Atom {
        if super::expr::interpolates(literal) {
            self.lost
                .borrow_mut()
                .push("a text literal with interpolation in a pattern (U4008)".to_owned());
            self.broken.set(true);
            return Atom::Unit;
        }
        self.literal_atom(lets, literal, span)
    }

    /// The label of a value computed from the whole value recorded at `span`, such as its text
    /// or a comparison with it: the deep join of its type's labels, kinds' fields included, as
    /// the checker labels it (S-27).
    /// A value with no recorded type fails closed: `⊤`, never `public`.
    fn computed_label(&self, span: Span) -> Label {
        self.records.types.get(&span).map_or_else(
            || Label::top(&self.env.categories),
            |ty| ty.computed_label(&self.env.kind_labels),
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
        // Each test depends on the value, so it carries its label (S-27).
        let label = self.computed_label(value_span);
        let above = self.temp();
        lets.push(Let::inferred(
            above.clone(),
            with_label(bool_ty(), &label),
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
            ty: with_label(bool_ty(), &label),
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
            with_label(bool_ty(), &label),
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
            ExprKind::Attempt { operand, within } => {
                // `attempt e within d`: `d` first, then `e` bounded by it (`spec/27 §5`).
                let mut lets = Vec::new();
                let deadline = within
                    .as_ref()
                    .map(|deadline| (self.atom(&mut lets, deadline), deadline.span));
                let mut body = self.expr(operand);
                if let Some((atom, deadline_span)) = deadline {
                    body = self.within_core(body, atom, deadline_span, false, expr.span);
                }
                close(
                    lets,
                    CoreExpr::Attempt {
                        body: Box::new(body),
                        span: expr.span,
                    },
                )
            }
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

    /// An operator of dense values at `span`, over `operands` in source order (ADR 0120 rule
    /// 34): the `map` or `zip_map` that its [`Res::DenseOp`] names, with a lambda that applies
    /// the element type's operator. Its parameters have the operands' element types,
    /// refinements included, and its result the result's element type, so a fixed-width
    /// element type carries the lambda's `overflow` obligation, over two values of the element
    /// type, and a division by a scalar its `division` obligation, once. A scalar operand is
    /// evaluated in its place and captured.
    fn dense_op(&mut self, lets: &mut Vec<Let>, span: Span, operands: &[&Expr]) -> Atom {
        let Some(Res::DenseOp { id, op }) = self.res(span) else {
            self.broken.set(true);
            return Atom::Unit;
        };
        let element = |ty: &Type| match ty {
            Type::Base {
                base: Base::Named { module, name, args },
                ..
            } if module == "prelude" && matches!(name.as_str(), "Array" | "Matrix" | "Tensor") => {
                args.first().cloned()
            }
            _ => None,
        };
        let mut dense = Vec::new();
        let mut scalar = None;
        let mut scalar_span = span;
        let mut arg_spans = Vec::new();
        for operand in operands {
            let ty = self.type_at(operand.span);
            let atom = self.atom(lets, operand);
            match element(&ty) {
                Some(element) => {
                    dense.push((atom, element));
                    arg_spans.push(operand.span);
                }
                None => {
                    scalar = Some(atom);
                    scalar_span = operand.span;
                }
            }
        }
        let Some(out) = element(&self.type_at(span)) else {
            self.broken.set(true);
            return Atom::Unit;
        };
        let float = matches!(
            dense.first(),
            Some((
                _,
                Type::Base {
                    base: Base::F64,
                    ..
                }
            ))
        );
        let params: Vec<(Name, Type)> = dense
            .iter()
            .map(|(_, element)| (self.temp(), element.clone()))
            .collect();
        let p = |index: usize| {
            params
                .get(index)
                .map_or(Atom::Unit, |(name, _)| Atom::Var(name.clone()))
        };
        let k = scalar.clone().unwrap_or(Atom::Unit);
        let (prim, args) = match (op, float) {
            (DenseOp::Add, true) => (Prim::Float(una_core::FloatOp::Add), vec![p(0), p(1)]),
            (DenseOp::Add, false) => (Prim::Add, vec![p(0), p(1)]),
            (DenseOp::Sub, true) => (Prim::Float(una_core::FloatOp::Sub), vec![p(0), p(1)]),
            (DenseOp::Sub, false) => (Prim::Sub, vec![p(0), p(1)]),
            (DenseOp::Neg, true) => (Prim::Float(una_core::FloatOp::Neg), vec![p(0)]),
            (DenseOp::Neg, false) => (Prim::Neg, vec![p(0)]),
            (DenseOp::Scale { scalar_first }, float) => {
                let prim = if float {
                    Prim::Float(una_core::FloatOp::Mul)
                } else {
                    Prim::Mul
                };
                let args = if scalar_first {
                    vec![k, p(0)]
                } else {
                    vec![p(0), k]
                };
                (prim, args)
            }
            (DenseOp::Divide, true) => (Prim::Float(una_core::FloatOp::Div), vec![p(0), k]),
            (DenseOp::Divide, false) => (Prim::Div, vec![p(0), k]),
        };
        // An element carries the operator's span, and the scalar its own (the verifier's
        // obligations name the divisor by it).
        let element_spans: Vec<Span> = args
            .iter()
            .map(|arg| {
                if scalar.as_ref() == Some(arg)
                    && !params.iter().any(|(n, _)| *arg == Atom::Var(n.clone()))
                {
                    scalar_span
                } else {
                    span
                }
            })
            .collect();
        let r = self.temp();
        let body = CoreExpr::Let {
            name: r.clone(),
            ty: out.clone(),
            value: Box::new(Bound::Rhs(Rhs::Prim {
                prim,
                args,
                arg_spans: element_spans,
                span,
            })),
            body: Box::new(CoreExpr::Atom {
                atom: Atom::Var(r),
                span,
            }),
            span,
            written: false,
        };
        let lambda_ty = Type::Fn {
            params: params.iter().map(|(_, ty)| ty.clone()).collect(),
            result: Box::new(out.clone()),
            uses: Vec::new(),
            fails: None,
            label: Label::public(),
        };
        let f = self.temp();
        lets.push(Let::inferred(
            f.clone(),
            lambda_ty,
            Bound::Rhs(Rhs::Lambda {
                params,
                body: Box::new(body),
                function: None,
            }),
            span,
        ));
        let mut type_args: Vec<Type> = dense.iter().map(|(_, element)| element.clone()).collect();
        type_args.push(out);
        let mut args: Vec<Atom> = dense.into_iter().map(|(atom, _)| atom).collect();
        args.push(Atom::Var(f));
        arg_spans.push(span);
        let function = self.function_path(id);
        self.push(
            lets,
            span,
            Rhs::Call {
                function,
                type_args,
                args,
                arg_spans,
                span,
            },
        )
    }

    /// Binds `rhs` to a fresh temporary of the type recorded at `span`.
    fn push(&mut self, lets: &mut Vec<Let>, span: Span, rhs: Rhs) -> Atom {
        let t = self.temp();
        let ty = self.type_at(span);
        lets.push(Let::inferred(t.clone(), ty, Bound::Rhs(rhs), span));
        Atom::Var(t)
    }

    /// Binds the call or operation `rhs` at `span` to a fresh temporary of the type recorded
    /// there, `quiet` when the checker marked it (S-39, `spec/26 §6` rule 4).
    fn push_call(&mut self, lets: &mut Vec<Let>, span: Span, rhs: Rhs) -> Atom {
        let t = self.temp();
        let ty = self.type_at(span);
        let value = self.quiet_call(rhs, &ty, span);
        lets.push(Let::inferred(t.clone(), ty, value, span));
        Atom::Var(t)
    }

    /// `expr`, `quiet` when the checker marked the construct `key` (S-39, `spec/26 §6` rule 4):
    /// a `within`, keyed by its deadline, a `first of`, or a `tasks`, `together`, `together
    /// for` or `start` statement, under a raised program counter.
    fn quiet_construct(&self, key: Span, expr: CoreExpr, span: Span) -> CoreExpr {
        if self.findings.quiet.contains(&key) {
            CoreExpr::Quiet {
                body: Box::new(expr),
                timing: !self.findings.hushed.contains(&key),
                span,
            }
        } else {
            expr
        }
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
        if let Some(atom) = self.folded(span) {
            return atom;
        }
        match &expr.kind {
            ExprKind::Old(_) => {
                self.broken.set(true);
                Atom::Unit
            }
            ExprKind::Literal(literal) => self.literal_atom(lets, literal, span),
            ExprKind::Paren(inner) => self.atom(lets, inner),
            ExprKind::Name(ident)
                if matches!(self.records.res.get(&ident.span), Some(Res::Const(_))) =>
            {
                let Some(Res::Const(id)) = self.records.res.get(&ident.span).cloned() else {
                    return self.unelaborated();
                };
                self.constant_atom(lets, id)
            }
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
                    Some(Res::Constant(value)) => Atom::Int(value.to_string()),
                    _ => self.unelaborated(),
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
                _ => self.unelaborated(),
            },
            ExprKind::List(items) => {
                let items: Vec<Atom> = items.iter().map(|item| self.atom(lets, item)).collect();
                self.push(lets, span, Rhs::List(items))
            }
            ExprKind::Binary { op, left, right } => self.binary(lets, *op, left, right, span),
            ExprKind::Unary { operand, .. }
                if matches!(self.records.res.get(&span), Some(Res::DenseOp { .. })) =>
            {
                self.dense_op(lets, span, &[operand])
            }
            ExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } if self.complex_at(operand.span).is_some() => {
                let value = self.atom(lets, operand);
                self.complex_unary(lets, ComplexStep::Neg, value, None, span)
            }
            ExprKind::Unary { op, operand } => {
                let operand_span = operand.span;
                let float = self.is_f64_at(operand_span);
                let decimal = self.decimal_at(operand_span).is_some();
                let float32 = self.is_f32_at(operand_span);
                let operand = self.atom(lets, operand);
                let prim = match op {
                    UnaryOp::Neg if float => Prim::Float(una_core::FloatOp::Neg),
                    UnaryOp::Neg if float32 => Prim::Float32(una_core::FloatOp::Neg),
                    UnaryOp::Neg if decimal => Prim::Decimal(una_core::DecimalOp::Neg),
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
                        _ => self.unelaborated(),
                    },
                    IsTarget::Literal(literal) => {
                        let literal = self.pattern_literal_atom(lets, literal, span);
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
            ExprKind::IfLine { .. } | ExprKind::Fail(_) | ExprKind::Attempt { .. } => {
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
                    _ => return self.unelaborated(),
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
                // `m.rows` and `m.columns` of a matrix (`spec/29 §17`).
                Some(Res::Shape(fun)) => {
                    let target_span = target.span;
                    let value = self.atom(lets, target);
                    self.push(
                        lets,
                        span,
                        Rhs::Prim {
                            prim: Prim::Shape(fun),
                            args: vec![value],
                            arg_spans: vec![target_span],
                            span,
                        },
                    )
                }
                // `b.size` of a bit set (`spec/28 §9`).
                Some(Res::BitSetMethod(op)) => {
                    let target_span = target.span;
                    let value = self.atom(lets, target);
                    self.push(
                        lets,
                        span,
                        Rhs::Prim {
                            prim: Prim::BitSet(op),
                            args: vec![value],
                            arg_spans: vec![target_span],
                            span,
                        },
                    )
                }
                Some(Res::Function { id, type_args }) => {
                    let receiver = self.atom(lets, target);
                    let receiver = (receiver, target.span);
                    self.call_function(lets, span, id, &type_args, Some(receiver), &[])
                }
                _ => self.unelaborated(),
            },
            ExprKind::MethodCall { target, args, .. } => match self.res(span) {
                // ADR 0104 rule 5: `E(x.measured())` and `E(x.magnitude())` are `E(x)`, and
                // `E(x.convert())` is `E(x) * k`, the receiver first, as `x * k` elaborates.
                Some(Res::UnitMethod { ratio: 1 }) => self.atom(lets, target),
                Some(Res::UnitMethod { ratio }) => {
                    let arg_spans = vec![target.span, span];
                    let left = self.atom(lets, target);
                    self.push(
                        lets,
                        span,
                        Rhs::Prim {
                            prim: Prim::Mul,
                            args: vec![left, Atom::Int(ratio.to_string())],
                            arg_spans,
                            span,
                        },
                    )
                }
                // ADR 0114 rule 12.7: `E(x.convert())` on an `F64` is `E(x) * k` or `E(x) / k`
                // with `k` the float literal of the ratio, and the same on an `F32`; on a
                // decimal, `E(x) * k` with `k` a `Decimal[0]` literal, which keeps the scale
                // (ADR 0120 rules 63 and 65).
                Some(Res::Convert {
                    ratio,
                    divide,
                    number,
                }) => {
                    let arg_spans = vec![target.span, span];
                    let left = self.atom(lets, target);
                    let op = if divide { FloatOp::Div } else { FloatOp::Mul };
                    let (prim, k) = match number {
                        ConvertNumber::F64 => {
                            (Prim::Float(op), Atom::F64(float_ratio(ratio).to_bits()))
                        }
                        ConvertNumber::F32 => {
                            (Prim::Float32(op), Atom::F32(float32_ratio(ratio).to_bits()))
                        }
                        ConvertNumber::Decimal(_) => (
                            Prim::Decimal(una_core::DecimalOp::Mul),
                            Atom::Decimal {
                                coefficient: ratio.to_string(),
                                scale: 0,
                            },
                        ),
                    };
                    self.push(
                        lets,
                        span,
                        Rhs::Prim {
                            prim,
                            args: vec![left, k],
                            arg_spans,
                            span,
                        },
                    )
                }
                // A built-in method of an integer type (`spec/18 §4`, `§5`), or one of a map, a
                // stack or a queue that reads it (`spec/21 §2`, `spec/28 §5`), or one of a set,
                // a hash map or a hash set that reads it or gives a new one (`spec/28 §6`, `§7`),
                // or one of a priority queue that reads it (`spec/28 §8`), or one of a bit set
                // that reads it or gives a new one (`spec/28 §9`).
                Some(
                    res @ (Res::IntMethod(_)
                    | Res::MapMethod(_)
                    | Res::SeqMethod(_)
                    | Res::SetMethod(_)
                    | Res::HashMapMethod(_)
                    | Res::HashSetMethod(_)
                    | Res::PriorityMethod(_)
                    | Res::BitSetMethod(_)),
                ) => {
                    let prim = match res {
                        Res::MapMethod(op) => Prim::Map(op),
                        Res::SeqMethod(op) => Prim::Seq(op),
                        Res::SetMethod(op) => Prim::Set(op),
                        Res::HashMapMethod(op) => Prim::HashMap(op),
                        Res::HashSetMethod(op) => Prim::HashSet(op),
                        Res::PriorityMethod(op) => Prim::Priority(op),
                        Res::BitSetMethod(op) => Prim::BitSet(op),
                        Res::IntMethod(prim) => prim,
                        _ => Prim::Len,
                    };
                    let mut arg_spans = vec![target.span];
                    let mut values = vec![self.atom(lets, target)];
                    for arg in args {
                        // A decimal method's `scale:` is static, part of its operation
                        // (`spec/29 §16`), and no operand.
                        if matches!(prim, Prim::Decimal(_))
                            && arg.label.as_ref().is_some_and(|l| self.name(l) == "scale")
                        {
                            continue;
                        }
                        arg_spans.push(arg.value.span);
                        values.push(self.atom(lets, &arg.value));
                    }
                    self.push(
                        lets,
                        span,
                        Rhs::Prim {
                            prim,
                            args: values,
                            arg_spans,
                            span,
                        },
                    )
                }
                // `x.conj()`, `x.scale(by: k)` and `x.norm_sqr()` (ADR 0120 rule 23).
                Some(Res::Complex(method)) => {
                    let value = self.atom(lets, target);
                    let factor = args.first().map(|arg| self.atom(lets, &arg.value));
                    let step = match method {
                        ComplexMethod::Conj => ComplexStep::Conj,
                        ComplexMethod::Scale => ComplexStep::Scale,
                        ComplexMethod::NormSqr => ComplexStep::NormSqr,
                    };
                    self.complex_unary(lets, step, value, factor, span)
                }
                // `x.complex(im: y)` (ADR 0120 rule 22).
                Some(Res::Ctor { kind, variant }) => {
                    let receiver = self.atom(lets, target);
                    let (args, arg_spans) =
                        self.ordered_values(lets, span, Some((receiver, target.span)), args);
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
                    self.push_call(
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
                _ => self.unelaborated(),
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
                    self.push_call(
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
                _ => self.unelaborated(),
            },
            ExprKind::CapOp { cap, op, args } => match self.res(span) {
                Some(Res::Op { capability, .. }) => {
                    let (values, arg_spans) = self.call_args(lets, span, args);
                    let labels: Vec<Label> = arg_spans
                        .iter()
                        .map(|arg| {
                            // An argument with no recorded type fails closed: `⊤`, so the trace
                            // redacts it.
                            self.records.types.get(arg).map_or_else(
                                || Label::top(&self.env.categories),
                                Ty::argument_label,
                            )
                        })
                        .collect();
                    // S-45 (#1552): the label of the outcome in a run's trace; an operation
                    // with no record fails closed at `⊤`, so the trace redacts its outcome.
                    let outcome = self.records.outcomes.get(&span).map_or_else(
                        || Label::top(&self.env.categories),
                        |parts| {
                            parts.iter().fold(Label::public(), |label, ty| {
                                label.join(&ty.argument_label())
                            })
                        },
                    );
                    let capability_path = self.capability_path(Some(capability));
                    let capability = self
                        .env
                        .capabilities
                        .get(capability)
                        .map_or("?", |c| c.name)
                        .to_owned();
                    self.push_call(
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
                            outcome,
                            span,
                        },
                    )
                }
                _ => self.unelaborated(),
            },
            // A placeholder with no binder here, as inside a lambda, which takes the binders
            // away, has no value in Core IR: the declaration is not elaborated, so the final
            // cross-check rejects it (U8001) instead of a `unit` standing in for it.
            ExprKind::Result => self.binder(self.result.clone()),
            ExprKind::It => self.binder(self.it.clone()),
            ExprKind::Error => self.binder(self.error.clone()),
            ExprKind::CapRef(_) => {
                self.broken.set(true);
                Atom::Unit
            }
        }
    }

    /// A form that resolved to something its syntax cannot elaborate to: the declaration is
    /// broken, and `unit` only fills the hole (`spec/09 §3`).
    fn unelaborated(&self) -> Atom {
        self.broken.set(true);
        Atom::Unit
    }

    /// The binder of `result`, `it` or `error` where one is in scope. Without one the form has
    /// no value to elaborate to: the declaration is broken, and `unit` only fills the hole.
    fn binder(&self, binder: Option<Name>) -> Atom {
        binder.map_or_else(|| self.unelaborated(), Atom::Var)
    }

    fn literal_atom(&mut self, lets: &mut Vec<Let>, literal: &Literal, span: Span) -> Atom {
        match literal {
            // ADR 0109 rule 3: an `Int` literal outside the tagged range in a `systems`
            // package is bound to its own temporary at its span, where the verifier finds its
            // `overflow` site. The value is the same, so no engine sees a difference.
            Literal::Int(digits)
                if self.env.in_package(self.module)
                    && self.findings.int_literals.contains(&span) =>
            {
                self.push(lets, span, Rhs::Atom(Atom::Int(digits.clone())))
            }
            Literal::Int(digits) => Atom::Int(digits.clone()),
            // A literal typed as a decimal is its coefficient at that scale (`spec/29 §16`).
            Literal::Float(text) if self.decimal_at(span).is_some() => {
                let scale = self.decimal_at(span).unwrap_or(0);
                match una_core::decimal::literal(text, scale) {
                    una_core::decimal::Literal::Value(coefficient) => Atom::Decimal {
                        coefficient: coefficient.to_string(),
                        scale,
                    },
                    // U2037, which the checker reported.
                    _ => {
                        self.broken.set(true);
                        Atom::Unit
                    }
                }
            }
            Literal::Float(text) if self.is_f32_at(span) => {
                match una_core::float32::literal(text) {
                    una_core::float32::Literal::Value(value) => Atom::F32(value.to_bits()),
                    // U2031, which the checker reported.
                    _ => {
                        self.broken.set(true);
                        Atom::F32(0)
                    }
                }
            }
            Literal::Float(text) => match una_core::float::literal(text) {
                una_core::float::Literal::Value(value) => Atom::F64(value.to_bits()),
                // U2031, which the checker reported.
                _ => {
                    self.broken.set(true);
                    Atom::F64(0)
                }
            },
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
                    // An `F64` is written by its shortest round-trip text (`spec/29 §7`).
                    let prim = if self.is_f64_at(expr.span) {
                        Prim::Float(una_core::FloatOp::ToText)
                    } else if let Some(scale) = self.decimal_at(expr.span) {
                        // A decimal is written with exactly its scale's digits (`spec/29 §16`).
                        Prim::Decimal(una_core::DecimalOp::ToText(scale))
                    } else if self.is_f32_at(expr.span) {
                        Prim::Float32(una_core::FloatOp::ToText)
                    } else {
                        Prim::ToText
                    };
                    let value = self.atom(lets, expr);
                    let t = self.temp();
                    // The text holds the value's fields in clear (`spec/10 §12` rule 14), so it
                    // carries their labels too, as the checker's label does (S-27).
                    let label = self.computed_label(expr.span);
                    lets.push(Let::inferred(
                        t.clone(),
                        with_label(text_ty(), &label),
                        Bound::Rhs(Rhs::Prim {
                            prim,
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
        // An operator of dense values: its `map` or `zip_map` (`spec/29 §17`).
        // A plain operator records nothing, so the lookup is not `res`, which would mark the
        // declaration broken.
        if matches!(self.records.res.get(&span), Some(Res::DenseOp { .. })) {
            return self.dense_op(lets, span, &[left, right]);
        }
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
        // The arithmetic of `Complex[F64]` and `Complex[F32]` is that of their components (ADR
        // 0120 rule 23); `==` and `!=` are the records' equality.
        if matches!(
            op,
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div
        ) && self.complex_at(span).is_some()
        {
            let x = self.atom(lets, left);
            let y = self.atom(lets, right);
            return self.complex_binary(lets, op, x, y, span);
        }
        let arg_spans = vec![left.span, right.span];
        let float = self.is_f64_at(left.span) && self.is_f64_at(right.span);
        let float32 = self.is_f32_at(left.span) && self.is_f32_at(right.span);
        let decimal = match (self.decimal_at(left.span), self.decimal_at(right.span)) {
            (Some(a), Some(b)) => decimal_op(op, a, b),
            _ => None,
        };
        let left = self.atom(lets, left);
        let right = self.atom(lets, right);
        let wrap = |prim: Prim| match prim {
            Prim::Float(op) if float32 => Prim::Float32(op),
            other => other,
        };
        let prim = match (op, float || float32) {
            // The operators of `Decimal` carry their operands' scales (`spec/29 §16`).
            _ if decimal.is_some() => Prim::Decimal(decimal.unwrap_or(una_core::DecimalOp::Mul)),
            // The operators of `F64` are their own primitives (ADR 0114 rule 17).
            (BinaryOp::Add, true) => Prim::Float(una_core::FloatOp::Add),
            (BinaryOp::Sub, true) => Prim::Float(una_core::FloatOp::Sub),
            (BinaryOp::Mul, true) => Prim::Float(una_core::FloatOp::Mul),
            (BinaryOp::Div, true) => Prim::Float(una_core::FloatOp::Div),
            (BinaryOp::Eq, true) => Prim::Float(una_core::FloatOp::Eq),
            (BinaryOp::NotEq, true) => Prim::Float(una_core::FloatOp::Ne),
            (BinaryOp::Lt, true) => Prim::Float(una_core::FloatOp::Lt),
            (BinaryOp::Le, true) => Prim::Float(una_core::FloatOp::Le),
            (BinaryOp::Gt, true) => Prim::Float(una_core::FloatOp::Gt),
            (BinaryOp::Ge, true) => Prim::Float(una_core::FloatOp::Ge),
            (op, _) => match op {
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
            },
        };
        // The same operators of `F32` are `Float32` primitives (ADR 0120 rule 5).
        let prim = wrap(prim);
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

    // ----- `Complex[F64]` and `Complex[F32]` -----

    /// The complex type recorded at `span`, its width (`true` for `Complex[F32]`), its label,
    /// and the record constructor of its kind.
    fn complex_shape(&mut self, span: Span) -> (Type, bool, Label, Ctor) {
        let ty = self.type_at(span);
        let (f32, label, ctor) = match &ty {
            Type::Base {
                base: Base::Named { module, name, .. },
                label,
                ..
            } => (
                name == una_core::complex::COMPLEX_F32,
                label.clone(),
                Ctor {
                    kind: Path {
                        module: module.clone(),
                        name: name.clone(),
                    },
                    name: name.clone(),
                    index: 0,
                },
            ),
            _ => {
                self.broken.set(true);
                (
                    false,
                    Label::public(),
                    Ctor {
                        kind: self.path(self.module, "?"),
                        name: "?".to_owned(),
                        index: 0,
                    },
                )
            }
        };
        (ty, f32, label, ctor)
    }

    /// Binds `rhs` to a fresh temporary of type `ty`.
    fn push_typed(&mut self, lets: &mut Vec<Let>, span: Span, ty: Type, rhs: Rhs) -> Atom {
        let t = self.temp();
        lets.push(Let::inferred(t.clone(), ty, Bound::Rhs(rhs), span));
        Atom::Var(t)
    }

    /// The components `re` and `im` of the complex value `value`, each read into a temporary.
    fn complex_parts(
        &mut self,
        lets: &mut Vec<Let>,
        parts: &ComplexParts,
        value: Atom,
    ) -> (Atom, Atom) {
        let re = self.push_typed(
            lets,
            parts.span,
            parts.component.clone(),
            Rhs::Field {
                value: value.clone(),
                field: "re".to_owned(),
                index: 0,
            },
        );
        let im = self.push_typed(
            lets,
            parts.span,
            parts.component.clone(),
            Rhs::Field {
                value,
                field: "im".to_owned(),
                index: 1,
            },
        );
        (re, im)
    }

    /// The operation `op` of the component type on `args`, bound to a temporary of type `ty`:
    /// one rounding, never fused with another (ADR 0120 rule 23).
    fn component_op(
        &mut self,
        lets: &mut Vec<Let>,
        parts: &ComplexParts,
        op: FloatOp,
        args: Vec<Atom>,
        ty: Type,
    ) -> Atom {
        let prim = if parts.f32 {
            Prim::Float32(op)
        } else {
            Prim::Float(op)
        };
        let arg_spans = vec![parts.span; args.len()];
        self.push_typed(
            lets,
            parts.span,
            ty,
            Rhs::Prim {
                prim,
                args,
                arg_spans,
                span: parts.span,
            },
        )
    }

    /// [`Fx::component_op`] with a result of the component type.
    fn component(
        &mut self,
        lets: &mut Vec<Let>,
        parts: &ComplexParts,
        op: FloatOp,
        args: Vec<Atom>,
    ) -> Atom {
        let ty = parts.component.clone();
        self.component_op(lets, parts, op, args, ty)
    }

    /// The binary operation `op` of the component type on `x` and `y`.
    fn op2(
        &mut self,
        lets: &mut Vec<Let>,
        parts: &ComplexParts,
        op: FloatOp,
        x: &Atom,
        y: &Atom,
    ) -> Atom {
        self.component(lets, parts, op, vec![x.clone(), y.clone()])
    }

    /// `complex(re: re, im: im)`, bound to a temporary of the complex type.
    fn complex_value(
        &mut self,
        lets: &mut Vec<Let>,
        parts: &ComplexParts,
        re: Atom,
        im: Atom,
    ) -> Atom {
        self.push_typed(
            lets,
            parts.span,
            parts.ty.clone(),
            Rhs::Construct {
                ctor: parts.ctor.clone(),
                args: vec![re, im],
                arg_spans: vec![parts.span, parts.span],
            },
        )
    }

    /// `-x`, `x.conj()`, `x.scale(by: k)` and `x.norm_sqr()` at `span`, by the operations of
    /// the components (ADR 0120 rule 23): O(1).
    fn complex_unary(
        &mut self,
        lets: &mut Vec<Let>,
        step: ComplexStep,
        value: Atom,
        factor: Option<Atom>,
        span: Span,
    ) -> Atom {
        // The receiver's type: the result of `norm_sqr()` is a component.
        let parts = self.complex_parts_at(span, step);
        let (a, b) = self.complex_parts(lets, &parts, value);
        match step {
            ComplexStep::Neg => {
                let re = self.component(lets, &parts, FloatOp::Neg, vec![a]);
                let im = self.component(lets, &parts, FloatOp::Neg, vec![b]);
                self.complex_value(lets, &parts, re, im)
            }
            ComplexStep::Conj => {
                let im = self.component(lets, &parts, FloatOp::Neg, vec![b]);
                self.complex_value(lets, &parts, a, im)
            }
            ComplexStep::Scale => {
                let Some(k) = factor else {
                    self.broken.set(true);
                    return Atom::Unit;
                };
                let re = self.component(lets, &parts, FloatOp::Mul, vec![a, k.clone()]);
                let im = self.component(lets, &parts, FloatOp::Mul, vec![b, k]);
                self.complex_value(lets, &parts, re, im)
            }
            ComplexStep::NormSqr => {
                let aa = self.component(lets, &parts, FloatOp::Mul, vec![a.clone(), a]);
                let bb = self.component(lets, &parts, FloatOp::Mul, vec![b.clone(), b]);
                self.component(lets, &parts, FloatOp::Add, vec![aa, bb])
            }
        }
    }

    /// The shape of the complex values of the operation `step` at `span`: the result's, or,
    /// for `norm_sqr()`, whose result is a component, the complex type of that component.
    fn complex_parts_at(&mut self, span: Span, step: ComplexStep) -> ComplexParts {
        if step != ComplexStep::NormSqr {
            return self.complex_parts_of(span);
        }
        let ty = self.type_at(span);
        let (f32, label) = match &ty {
            Type::Base {
                base: Base::F32,
                label,
                ..
            } => (true, label.clone()),
            Type::Base { label, .. } => (false, label.clone()),
            Type::Fn { .. } => (false, Label::public()),
        };
        let name = if f32 {
            una_core::complex::COMPLEX_F32
        } else {
            una_core::complex::COMPLEX_F64
        };
        let kind = Path {
            module: una_core::PRELUDE_MODULE.to_owned(),
            name: name.to_owned(),
        };
        ComplexParts {
            ty: with_label(
                plain(Base::Named {
                    module: kind.module.clone(),
                    name: kind.name.clone(),
                    args: Vec::new(),
                }),
                &label,
            ),
            component: ty,
            f32,
            label,
            ctor: Ctor {
                kind,
                name: name.to_owned(),
                index: 0,
            },
            span,
        }
    }

    /// The shape of the complex value of the expression at `span`.
    fn complex_parts_of(&mut self, span: Span) -> ComplexParts {
        let (ty, f32, label, ctor) = self.complex_shape(span);
        let base = if f32 { Base::F32 } else { Base::F64 };
        ComplexParts {
            ty,
            component: with_label(plain(base), &label),
            f32,
            label,
            ctor,
            span,
        }
    }

    /// `x + y`, `x - y`, `x * y` and `x / y` of two complex values at `span`, by the operations
    /// of the components in the order of ADR 0120 rule 23, with no contraction; division is
    /// Smith's algorithm, with its two branches chosen by `c.abs().ieee_ge(other: d.abs())`.
    /// O(1).
    fn complex_binary(
        &mut self,
        lets: &mut Vec<Let>,
        op: BinaryOp,
        x: Atom,
        y: Atom,
        span: Span,
    ) -> Atom {
        let parts = self.complex_parts_of(span);
        let (a, b) = self.complex_parts(lets, &parts, x);
        let (c, d) = self.complex_parts(lets, &parts, y);
        match op {
            BinaryOp::Add | BinaryOp::Sub => {
                let float = if op == BinaryOp::Add {
                    FloatOp::Add
                } else {
                    FloatOp::Sub
                };
                let re = self.component(lets, &parts, float, vec![a, c]);
                let im = self.component(lets, &parts, float, vec![b, d]);
                self.complex_value(lets, &parts, re, im)
            }
            BinaryOp::Mul => {
                // `re = a * c - b * d`, `im = a * d + b * c`.
                let ac = self.op2(lets, &parts, FloatOp::Mul, &a, &c);
                let bd = self.op2(lets, &parts, FloatOp::Mul, &b, &d);
                let re = self.op2(lets, &parts, FloatOp::Sub, &ac, &bd);
                let ad = self.op2(lets, &parts, FloatOp::Mul, &a, &d);
                let bc = self.op2(lets, &parts, FloatOp::Mul, &b, &c);
                let im = self.op2(lets, &parts, FloatOp::Add, &ad, &bc);
                self.complex_value(lets, &parts, re, im)
            }
            _ => {
                let c_abs = self.component(lets, &parts, FloatOp::Abs, vec![c.clone()]);
                let d_abs = self.component(lets, &parts, FloatOp::Abs, vec![d.clone()]);
                let flag = with_label(bool_ty(), &parts.label);
                let wide =
                    self.component_op(lets, &parts, FloatOp::IeeeGe, vec![c_abs, d_abs], flag);
                // `|c| >= |d|`: `t = d / c`, `n = c + d * t`, `re = (a + b * t) / n`,
                // `im = (b - a * t) / n`.
                let mut first = Vec::new();
                let t = self.op2(&mut first, &parts, FloatOp::Div, &d, &c);
                let dt = self.op2(&mut first, &parts, FloatOp::Mul, &d, &t);
                let n = self.op2(&mut first, &parts, FloatOp::Add, &c, &dt);
                let bt = self.op2(&mut first, &parts, FloatOp::Mul, &b, &t);
                let p = self.op2(&mut first, &parts, FloatOp::Add, &a, &bt);
                let re = self.op2(&mut first, &parts, FloatOp::Div, &p, &n);
                let at = self.op2(&mut first, &parts, FloatOp::Mul, &a, &t);
                let q = self.op2(&mut first, &parts, FloatOp::Sub, &b, &at);
                let im = self.op2(&mut first, &parts, FloatOp::Div, &q, &n);
                let r = self.complex_value(&mut first, &parts, re, im);
                // Otherwise: `t = c / d`, `n = c * t + d`, `re = (a * t + b) / n`,
                // `im = (b * t - a) / n`.
                let mut second = Vec::new();
                let t = self.op2(&mut second, &parts, FloatOp::Div, &c, &d);
                let ct = self.op2(&mut second, &parts, FloatOp::Mul, &c, &t);
                let n = self.op2(&mut second, &parts, FloatOp::Add, &ct, &d);
                let at = self.op2(&mut second, &parts, FloatOp::Mul, &a, &t);
                let p = self.op2(&mut second, &parts, FloatOp::Add, &at, &b);
                let re = self.op2(&mut second, &parts, FloatOp::Div, &p, &n);
                let bt = self.op2(&mut second, &parts, FloatOp::Mul, &b, &t);
                let q = self.op2(&mut second, &parts, FloatOp::Sub, &bt, &a);
                let im = self.op2(&mut second, &parts, FloatOp::Div, &q, &n);
                let s = self.complex_value(&mut second, &parts, re, im);
                let quotient = CoreExpr::If {
                    cond: wide,
                    then: Box::new(close(first, CoreExpr::Atom { atom: r, span })),
                    otherwise: Box::new(close(second, CoreExpr::Atom { atom: s, span })),
                };
                let t = self.temp();
                bind_expr(lets, t.clone(), parts.ty.clone(), quotient, span, false);
                Atom::Var(t)
            }
        }
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
            self.broken.set(true);
            return Atom::Unit;
        };
        // A tensor's shape, index or permutation: the elements of its list literal are the
        // arguments, as the checker bound them (`spec/29 §17`).
        let spread_args: Vec<Arg>;
        let args = match self.records.spread.get(&span) {
            Some(position) => {
                spread_args = super::dense::spread(args, *position);
                spread_args.as_slice()
            }
            None => args,
        };
        let (values, arg_spans) = self.ordered_values(lets, span, receiver, args);
        // `json_decimal(x)` writes `x` at its scale, which only its type knows: an operation
        // of `Decimal` with that scale (`spec/29 §16`).
        if def.module == self.env.prelude
            && def.name == "json_decimal"
            && let Some(scale) = arg_spans.first().and_then(|at| self.decimal_at(*at))
        {
            return self.push(
                lets,
                span,
                Rhs::Prim {
                    prim: Prim::Decimal(una_core::DecimalOp::ToJson(scale)),
                    args: values,
                    arg_spans,
                    span,
                },
            );
        }
        if Some(id) == self.env.map_update {
            return self.map_update(lets, span, type_args, values, arg_spans, false);
        }
        if Some(id) == self.env.hash_map_update {
            return self.map_update(lets, span, type_args, values, arg_spans, true);
        }
        if def.kind == FnKind::Policy {
            let FnAst::Policy(policy) = def.ast else {
                self.broken.set(true);
                return Atom::Unit;
            };
            let via_name = self.env.text(def.module, policy.via.name);
            let via = match self.env.lower(def.module, via_name) {
                Some(Entity::Function(via)) => self.function_path(via),
                _ => {
                    self.broken.set(true);
                    return Atom::Unit;
                }
            };
            let arg = values.into_iter().next().unwrap_or(Atom::Unit);
            let arg_span = arg_spans.first().copied().unwrap_or(span);
            // A call of the policy's `via`, a check point as any call (`spec/27 §8` rule 2),
            // elaborated as one (#2214).
            return self.push_call(
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
        // ADR 0120: the type arguments of a call of a function that computes new elements keep
        // no refinement (`super::dense::SYNTHESIZING`), so the verifier derives its result, and
        // the new values of its `edit` places, without the element type's refinement: a
        // binding or a place whose declared type claims it gets an obligation that the call's
        // facts cannot prove, and the program is refused, never accepted on a false fact.
        let synthesizes = super::dense::synthesizes(self.env.prelude, def.module, def.name);
        let type_args: Vec<Type> = type_args
            .iter()
            .map(|t| {
                if synthesizes {
                    self.ty(&t.without_refinements())
                } else {
                    self.ty(t)
                }
            })
            .collect();
        let function = self.function_path(id);
        let call = Rhs::Call {
            function,
            type_args,
            args: values,
            arg_spans,
            span,
        };
        // A call with `edit` arguments returns a tuple (`spec/17 §4`), which only the desugar
        // of a statement or `let` binds. Anywhere else, its tuple would pass as the declared
        // result and its places would keep their values: an internal error (U8001), never a
        // silent pass.
        let edits = def
            .params
            .iter()
            .any(|p| p.convention == Some(Convention::Edit));
        match self.call_tuple.take() {
            Some((at, ty)) if at == span => {
                let t = self.temp();
                let value = self.quiet_call(call, &ty, span);
                lets.push(Let::inferred(t.clone(), ty, value, span));
                Atom::Var(t)
            }
            other => {
                self.call_tuple = other;
                if edits {
                    self.lost.borrow_mut().push(
                        "the tuple of a call with `edit` arguments outside a statement or `let`"
                            .to_owned(),
                    );
                    self.broken.set(true);
                }
                let t = self.temp();
                let ty = self.type_at(span);
                let value = self.quiet_call(call, &ty, span);
                lets.push(Let::inferred(t.clone(), ty, value, span));
                Atom::Var(t)
            }
        }
    }

    /// The value of a `let` of `call`, of type `ty`: the call itself, or, for a call or an
    /// operation that the checker marked under a raised program counter, `quiet (let t = call in
    /// t)` (S-39, `spec/26 §6` rule 4).
    fn quiet_call(&mut self, call: Rhs, ty: &Type, span: Span) -> Bound {
        if !self.findings.quiet.contains(&span) {
            return Bound::Rhs(call);
        }
        let t = self.temp();
        let timing = !self.findings.hushed.contains(&span);
        Bound::Expr(CoreExpr::Quiet {
            timing,
            body: Box::new(CoreExpr::Let {
                name: t.clone(),
                ty: ty.clone(),
                value: Box::new(Bound::Rhs(call)),
                body: Box::new(CoreExpr::Atom {
                    atom: Atom::Var(t),
                    span,
                }),
                span,
                written: false,
            }),
            span,
        })
    }

    /// `edit m.update(key: k, initial: i, change: f)` on a map, edition 0.5 (ADR 0119 rule
    /// 18): `v = when m.get(k) is Some(x) -> x, None -> i`, then `m.with(k, f(v))`, as the
    /// tuple `(m', unit)` of a call with an `edit` argument (`spec/17 §4`). `f` is called
    /// exactly once, and a failure of it leaves `m` as it was, since the place takes its new
    /// value only when the call returns (`spec/17 §7` rule 1). A lookup and an insertion,
    /// O(log n) each (`spec/21 §5`). `values` and `arg_spans` are in parameter order. With
    /// `hashed`, the same on a hash map (ADR 0119 rule 12), O(|k|) expected each.
    fn map_update(
        &mut self,
        lets: &mut Vec<Let>,
        span: Span,
        type_args: &[Ty],
        values: Vec<Atom>,
        arg_spans: Vec<Span>,
        hashed: bool,
    ) -> Atom {
        let (get, with) = if hashed {
            (
                Prim::HashMap(una_core::HashMapOp::Get),
                Prim::HashMap(una_core::HashMapOp::With),
            )
        } else {
            (Prim::Map(MapOp::Get), Prim::Map(MapOp::With))
        };
        let ([map, key, initial, change], [map_span, key_span, initial_span, change_span]) =
            (values.as_slice(), arg_spans.as_slice())
        else {
            self.broken.set(true);
            return Atom::Unit;
        };
        // The receiver's type as the checker recorded it, else `Map[K, V]` at the call's type
        // arguments.
        let map_ty = match self.records.types.get(map_span) {
            Some(ty) => self.ty(ty),
            None => {
                let arg = |index: usize| type_args.get(index).cloned().unwrap_or_else(Ty::error);
                if hashed {
                    self.ty(&Ty::Coll(
                        una_core::Coll::HashMap,
                        vec![arg(0), arg(1)].into(),
                    ))
                } else {
                    self.ty(&Ty::Map(
                        super::ty::Shared::new(arg(0)),
                        super::ty::Shared::new(arg(1)),
                    ))
                }
            }
        };
        let value_ty = match &map_ty {
            Type::Base {
                base: Base::Named { args, .. },
                ..
            } => args.get(1).cloned().unwrap_or_else(unknown),
            _ => unknown(),
        };
        let option_ty = Type::Base {
            base: Base::Named {
                module: una_core::PRELUDE_MODULE.to_owned(),
                name: "Option".to_owned(),
                args: vec![value_ty.clone()],
            },
            refinements: Vec::new(),
            label: Label::public(),
        };
        let found = self.temp();
        lets.push(Let::inferred(
            found.clone(),
            option_ty.clone(),
            Bound::Rhs(Rhs::Prim {
                prim: get,
                args: vec![map.clone(), key.clone()],
                arg_spans: vec![*map_span, *key_span],
                span,
            }),
            span,
        ));
        let current = self.unwrap_option(
            CoreExpr::Atom {
                atom: Atom::Var(found),
                span,
            },
            option_ty,
            CoreExpr::Atom {
                atom: initial.clone(),
                span: *initial_span,
            },
            span,
        );
        let before = self.temp();
        bind_expr(lets, before.clone(), value_ty.clone(), current, span, false);
        let after = self.temp();
        // S-39: `change` may run any construct, `quiet` under a raised program counter.
        let changed = self.quiet_call(
            Rhs::Apply {
                callee: change.clone(),
                args: vec![Atom::Var(before)],
                arg_spans: vec![*initial_span],
                span: *change_span,
            },
            &value_ty,
            span,
        );
        lets.push(Let::inferred(after.clone(), value_ty, changed, span));
        let updated = self.temp();
        lets.push(Let::inferred(
            updated.clone(),
            map_ty,
            Bound::Rhs(Rhs::Prim {
                prim: with,
                args: vec![map.clone(), key.clone(), Atom::Var(after)],
                arg_spans: vec![*map_span, *key_span, *change_span],
                span,
            }),
            span,
        ));
        match self.call_tuple.take() {
            Some((at, ty)) if at == span => {
                let t = self.temp();
                lets.push(Let::inferred(
                    t.clone(),
                    ty,
                    Bound::Rhs(Rhs::Tuple(vec![Atom::Var(updated), Atom::Unit])),
                    span,
                ));
                Atom::Var(t)
            }
            other => {
                self.call_tuple = other;
                self.lost.borrow_mut().push(
                    "the tuple of a call with `edit` arguments outside a statement or `let`"
                        .to_owned(),
                );
                self.broken.set(true);
                Atom::Unit
            }
        }
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
            self.broken.set(true);
        }
        let count = order.iter().flatten().max().map_or(0, |max| max + 1);
        let mut slots: Vec<Option<(Atom, Span)>> = vec![None; count.max(written.len())];
        for (value, target) in written.into_iter().zip(order.iter().copied()) {
            match target.and_then(|t| slots.get_mut(t)) {
                Some(slot) => *slot = Some(value),
                None => self.broken.set(true),
            }
        }
        slots.into_iter().flatten().unzip()
    }

    /// A function used as a value at `span`: `λ(x, ...). f[τ*](x, ...)`, whose parameters keep
    /// the declared names and types, refinements included (ADR 0037 rule 1).
    fn function_value(&mut self, id: usize, type_args: &[Ty], span: Span) -> Rhs {
        let Some(def) = self.env.functions.get(id) else {
            self.broken.set(true);
            return Rhs::Atom(Atom::Unit);
        };
        // A function that computes new elements is U2001 as a value; should that ever change,
        // its call here keeps no refinement of the element type either, as every call does.
        let stripped: Vec<Ty>;
        let type_args = if super::dense::synthesizes(self.env.prelude, def.module, def.name) {
            stripped = type_args.iter().map(Ty::without_refinements).collect();
            stripped.as_slice()
        } else {
            type_args
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
        .fold(tail, |body, binding| match binding.destructure {
            Some(names) => CoreExpr::LetTuple {
                names,
                value: match binding.value {
                    Bound::Rhs(Rhs::Atom(atom)) => atom,
                    _ => Atom::Unit,
                },
                body: Box::new(body),
                span: binding.span,
            },
            None => CoreExpr::Let {
                name: binding.name,
                ty: binding.ty,
                value: Box::new(binding.value),
                body: Box::new(body),
                span: binding.span,
                written: binding.written,
            },
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
                    destructure: None,
                });
                current = *body;
            }
            CoreExpr::LetTuple {
                names,
                value,
                body,
                span: inner_span,
            } => {
                lets.push(Let::destructure(names, value, inner_span));
                current = *body;
            }
            // ADR 0037 rule 2: an annotated binder whose function type differs from the type
            // of the value keeps the temporary, so the flow from one to the other is an
            // `Atom::Var` right side that the verifier checks, whatever the value is.
            // Only a temporary is merged: a source binder stays in scope for the code after
            // it, so `let b = a` keeps `a` (#139).
            CoreExpr::Atom {
                atom: Atom::Var(var),
                ..
            } if var.starts_with('%')
                && lets.last().is_some_and(|last| {
                    last.destructure.is_none()
                        && last.name == var
                        && !(written && last.ty != ty && has_fn_type(&ty))
                        // The temporary of an `Int` literal outside the tagged range, which a
                        // `systems` package binds, keeps its own span, the literal's
                        // `overflow` site (ADR 0109 rule 3).
                        && !matches!(
                            &last.value,
                            Bound::Rhs(Rhs::Atom(Atom::Int(digits)))
                                if !super::fixed::fits_word(false, digits)
                        )
                }) =>
            {
                if let Some(last) = lets.pop() {
                    lets.push(Let {
                        name,
                        ty,
                        value: last.value,
                        span,
                        written,
                        destructure: None,
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
                    destructure: None,
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
                    destructure: None,
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
        BlockExpr::TogetherFor(together) => together.span,
        BlockExpr::FirstOf(first) => first.span,
    }
}

/// The text of a literal without interpolation.
/// The term of a folded literal of a module constant (ADR 0126 rule 8), where the refinement
/// language has one; a decimal is its coefficient (`spec/29 §16`).
fn atom_term(atom: Atom) -> Option<Term> {
    Some(match atom {
        Atom::Int(digits) => Term::Int(digits),
        Atom::Bool(b) => Term::Bool(b),
        Atom::Text(text) => Term::Text(text),
        Atom::Unit => Term::Unit,
        Atom::F64(bits) => Term::F64(bits),
        Atom::F32(bits) => Term::F32(bits),
        Atom::Decimal { coefficient, .. } => Term::Int(coefficient),
        Atom::Var(_) => return None,
    })
}

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
    // Units are erased by design (ADR 0104 rule 5), so a unit is never a lost component. A
    // type without one is not rebuilt.
    let erased;
    let ty = if ty.has_units() {
        erased = ty.without_units();
        &erased
    } else {
        ty
    };
    let label = match core {
        Type::Base { label, .. } | Type::Fn { label, .. } => label,
    };
    if matches!(ty.unlabeled(), Ty::Var(_) | Ty::Never | Ty::Error(_)) {
        return None;
    }
    // A collection's Core label also holds the label its shape implies (ADR 0119 rule 3),
    // which only raises it.
    let kept = match ty.unlabeled() {
        Ty::Coll(..) => ty.top_label().flows_to(label),
        _ => *label == ty.top_label(),
    };
    if !kept {
        return Some(format!("the label `{}` became `{label}`", ty.top_label()));
    }
    // ADR 0103: every refinement of a nested position is in Core IR.
    if let Some(refinements) = ty.refinements() {
        let kept = match core {
            Type::Base { refinements, .. } => refinements.len(),
            Type::Fn { .. } => 0,
        };
        if kept < refinements.len() {
            return Some("a refinement of a type argument".to_owned());
        }
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
            Ty::List(element) | Ty::Dense(_, element),
            Type::Base {
                base: Base::Named { args, .. },
                ..
            },
        ) => args.first().and_then(|core| lost(element, core)),
        (
            Ty::Map(key, value),
            Type::Base {
                base: Base::Named { args, .. },
                ..
            },
        ) => [key.as_ref(), value.as_ref()]
            .into_iter()
            .zip(args)
            .find_map(|(ty, core)| lost(ty, core)),
        (
            Ty::Kind(_, args) | Ty::Coll(_, args),
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

/// The label that the shape of the collection type `ty` implies on its top (ADR 0119 rule 3,
/// IF2 to IF4): every label of the arguments that are part of its shape, the labels that
/// their kinds' fields declare included; `public` for any other type.
fn implied_label(ty: &Ty, kinds: &super::ty::KindLabels) -> Label {
    match ty.unlabeled() {
        Ty::Coll(coll, args) => args
            .iter()
            .enumerate()
            .filter(|(index, _)| coll.is_shape(*index))
            .fold(Label::public(), |label, (_, arg)| {
                label.join(&arg.every_label(kinds))
            }),
        _ => Label::public(),
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

/// Records an update of `name`, unless it is declared in the construct.
fn note_update(name: &str, declared: &[String], found: &mut Vec<String>) {
    if !declared.iter().any(|d| d == name) && !found.iter().any(|f| f == name) {
        found.push(name.to_owned());
    }
}

/// The Core tuple type of `components` (`spec/17 §4`).
fn tuple_type(components: Vec<Type>) -> Type {
    plain(Base::Tuple(components))
}

/// `ty` without its refinements: a value whose refinement is still to be proven.
fn strip(ty: Type) -> Type {
    match ty {
        Type::Base { base, label, .. } => Type::Base {
            base,
            refinements: Vec::new(),
            label,
        },
        other => other,
    }
}

fn has_refinements(ty: &Type) -> bool {
    matches!(ty, Type::Base { refinements, .. } if !refinements.is_empty())
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

/// The constraints of a function's `count` type parameters, as Core IR states them, for the
/// type descriptors of the native backend (`spec/20 §7`).
fn core_constraints(constraints: &[Constraint], count: usize) -> Vec<una_core::Constraint> {
    (0..count)
        .map(
            |index| match constraints.get(index).copied().unwrap_or_default() {
                Constraint::None => una_core::Constraint::None,
                Constraint::Eq => una_core::Constraint::Eq,
                Constraint::Ord => una_core::Constraint::Ord,
                Constraint::Integer => una_core::Constraint::Integer,
            },
        )
        .collect()
}

/// A failure injected into elaboration, compiled only in tests (#1050): while a [`Break`] is
/// held on this thread, the declaration, standard signature or example of its name does not
/// elaborate, and no diagnostic says so, as a bug of the elaboration would leave it.
#[cfg(test)]
pub(crate) mod hook {
    use std::cell::RefCell;

    use std::cell::Cell;

    thread_local! {
        static BROKEN: RefCell<Option<String>> = const { RefCell::new(None) };
        static EAGER: Cell<bool> = const { Cell::new(false) };
    }

    /// An injected failure, until dropped.
    pub(crate) struct Break;

    impl Break {
        /// Breaks the elaboration of the declaration named `name`.
        pub(crate) fn of(name: &str) -> Break {
            BROKEN.with(|broken| *broken.borrow_mut() = Some(name.to_owned()));
            Break
        }

        /// Looks every name of a formula up as a resolution, a local's too, whose span the
        /// records never hold: [`super::Fx::res`] then finds nothing and marks the declaration
        /// broken without a diagnostic, the regression found while adding the host-driven step
        /// (#1050).
        pub(crate) fn eager_resolution() -> Break {
            EAGER.with(|eager| eager.set(true));
            Break
        }
    }

    impl Drop for Break {
        fn drop(&mut self) {
            BROKEN.with(|broken| *broken.borrow_mut() = None);
            EAGER.with(|eager| eager.set(false));
        }
    }

    pub(super) fn breaks(name: &str) -> bool {
        BROKEN.with(|broken| broken.borrow().as_deref() == Some(name))
    }

    pub(super) fn eager() -> bool {
        EAGER.with(Cell::get)
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
            gives: Vec::new(),
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
    fn a_pattern_literal_that_interpolates_is_found_for_u8001() {
        // S-28: the checks reject it (U4008); the elaboration's guard keys on the same test.
        let mut db = una_syntax::SourceDb::new();
        let file = db.add("a.una", b"x");
        let span = Span::at(file, 0);
        let text = |parts| Literal::Text(TextLit { parts, span });
        let value = Expr {
            kind: ExprKind::Literal(Literal::Bool(true)),
            span,
        };
        assert!(super::super::expr::interpolates(&text(vec![
            TextPart::Text("a".to_owned()),
            TextPart::Interpolation(value),
        ])));
        assert!(!super::super::expr::interpolates(&text(vec![
            TextPart::Text("a".to_owned())
        ])));
        assert!(!super::super::expr::interpolates(&Literal::Int(
            "3".to_owned()
        )));
    }

    #[test]
    fn the_consistency_check_finds_lost_uses_fails_and_labels() {
        let secret = Label {
            level: una_core::Level::Secret,
            categories: una_core::Names::new(),
            symbols: una_core::Names::new(),
        };
        let function = Ty::Fn(super::super::ty::Shared::new(super::super::ty::FnTy {
            params: vec![Ty::Text],
            result: Ty::Unit,
            uses: vec![logging()],
            fails: Some(Ty::Kind(0, Vec::new().into())),
            refinements: super::super::ty::FnRefinements::default(),
        }));
        let kept = core_fn(
            vec![Use {
                cap: "log".to_owned(),
                capability: "Log".to_owned(),
                ops: vec!["write".to_owned()],
                gives: Vec::new(),
                conds: Vec::new(),
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
                gives: Vec::new(),
                conds: Vec::new(),
            }],
            None,
            Label::public(),
        );
        assert!(lost(&function, &no_fails).is_some());
        let listed = Ty::List(super::super::ty::Shared::new(
            function.clone().with_label(&secret),
        ));
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
        let function = Ty::Fn(super::super::ty::Shared::new(super::super::ty::FnTy {
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

/// The source names in scope during elaboration and their Core IR binders, innermost last, with
/// an index from each name to its positions, so a name resolves in expected O(1) whatever the
/// number of locals (#326). The index is only looked up, never iterated.
#[derive(Default)]
struct Scope {
    items: Vec<(String, Name)>,
    index: std::collections::HashMap<String, Vec<usize>>,
}

impl Scope {
    fn push(&mut self, (source, binder): (String, Name)) {
        self.index
            .entry(source.clone())
            .or_default()
            .push(self.items.len());
        self.items.push((source, binder));
    }

    fn pop(&mut self) {
        if let Some((source, _)) = self.items.pop()
            && let Some(positions) = self.index.get_mut(&source)
        {
            positions.pop();
            if positions.is_empty() {
                self.index.remove(&source);
            }
        }
    }

    fn truncate(&mut self, len: usize) {
        while self.items.len() > len {
            self.pop();
        }
    }

    fn len(&self) -> usize {
        self.items.len()
    }

    /// The binder of the innermost source name `name`.
    fn find(&self, name: &str) -> Option<Name> {
        let position = self.index.get(name)?.last()?;
        self.items.get(*position).map(|(_, binder)| binder.clone())
    }
}

/// The operation of `Decimal` for the operator `op` on operands of scales `a` and `b`
/// (`spec/29 §16`, ADR 0120 rule 13); `None` for an operator decimals do not have.
/// The ratio of a `convert()` on an `F64`, at most 2^53, so exact (ADR 0114 rule 12.5).
fn float_ratio(ratio: u128) -> f64 {
    ratio as f64
}

/// The ratio of a `convert()` on an `F32`, at most 2^24, so exact (ADR 0120 rule 62).
fn float32_ratio(ratio: u128) -> f32 {
    ratio as f32
}

fn decimal_op(op: BinaryOp, a: u8, b: u8) -> Option<una_core::DecimalOp> {
    use una_core::DecimalOp;
    Some(match op {
        BinaryOp::Add => DecimalOp::Add(a, b),
        BinaryOp::Sub => DecimalOp::Sub(a, b),
        BinaryOp::Mul => DecimalOp::Mul,
        BinaryOp::Eq => DecimalOp::Eq(a, b),
        BinaryOp::NotEq => DecimalOp::Ne(a, b),
        BinaryOp::Lt => DecimalOp::Lt(a, b),
        BinaryOp::Le => DecimalOp::Le(a, b),
        BinaryOp::Gt => DecimalOp::Gt(a, b),
        BinaryOp::Ge => DecimalOp::Ge(a, b),
        _ => return None,
    })
}

/// `term` times the literal 10^k, a coefficient aligned to a scale k larger (ADR 0120 V2).
fn aligned(term: Term, k: u8) -> Term {
    if k == 0 {
        return term;
    }
    Term::Binary {
        op: BinOp::Mul,
        left: Box::new(Term::Int(
            una_core::decimal::pow10(u32::from(k)).to_string(),
        )),
        right: Box::new(term),
    }
}

#[cfg(test)]
mod server_kinds {
    #[test]
    fn the_server_kinds_of_core_are_the_server_names_with_a_declaration_of_their_own() {
        // ADR 0123 rule 1: every name of the server but the capability, the refined alias
        // `Status` and the built-in `ReplyRight` is a kind that Core IR declares.
        let kinds: Vec<&str> = crate::names::prelude::SERVER_NAMES
            .iter()
            .copied()
            .filter(|name| !matches!(*name, "HttpServer" | "Status" | "ReplyRight"))
            .collect();
        assert_eq!(kinds, super::SERVER_CORE_KINDS);
    }

    /// The quoted names of the Lean definition `def <name> : List String := [...]` in `text`.
    fn lean_list(text: &str, name: &str) -> Vec<String> {
        let head = format!("def {name} : List String := [");
        let start = text.find(&head).map(|i| i + head.len());
        let Some(start) = start else {
            return Vec::new();
        };
        let end = text[start..].find(']').map_or(start, |i| start + i);
        text[start..end]
            .split(',')
            .map(|item| item.trim().trim_matches('"').to_owned())
            .filter(|item| !item.is_empty())
            .collect()
    }

    #[test]
    fn the_formal_importer_refuses_every_name_of_the_server() {
        // #1012: the formal re-check refuses the server's constructs by name until F1 models them
        // (ADR 0123 S10, `spec/31 §9`). Its lists must be the checker's, so a kind of the server
        // that Core gains is refused there too, never read as a record of the program.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../formal/UnaFormal/F1/Import.lean"
        );
        let text = std::fs::read_to_string(path).unwrap_or_default();
        assert!(!text.is_empty(), "cannot read {path}");
        assert_eq!(lean_list(&text, "serverKinds"), super::SERVER_CORE_KINDS);
        assert_eq!(
            lean_list(&text, "serverTestingNames"),
            crate::names::prelude::STD_TESTING_SERVER_NAMES
        );
        assert!(text.contains("def serverCapability : String := \"HttpServer\""));
        // Every other name of the server is the capability, `ReplyRight` (the base type the
        // importer refuses by its node) or `Status` (an `Int` with a refinement in Core).
        for name in crate::names::prelude::SERVER_NAMES {
            assert!(
                super::SERVER_CORE_KINDS.contains(&name)
                    || matches!(name, "HttpServer" | "ReplyRight" | "Status"),
                "`{name}` is a name of the server the formal importer does not refuse"
            );
        }
        assert!(text.contains("| .str \"reply_right\", _, _, _ =>"));
    }
}
