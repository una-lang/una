//! The report's `preview_constructs` (ADR 0121 rule 8, `spec/13 §6`): each construct of the
//! edition ledger (rule 9) that the root package uses, with its number of sites.
//!
//! A site is a place of the root package where the construct is written, counted once however
//! many times elaboration repeats it. Each construct is counted where it can be seen for sure:
//!
//! - **Core IR**, for the forms that elaboration keeps: the loops, `stop` and `skip`, `within`,
//!   `first of`, `tasks` and `start`, an installed `limiter`, the operations of `HttpServer`
//!   and `Limit`, the dense kernels by their prelude function, the prelude functions and
//!   standard declarations that an entry adds, the variants an entry adds where a `when` names
//!   them, a test's `stop after`, and every type that an entry adds, wherever a binding, a
//!   signature or a declaration holds one;
//! - **the checker**, for a form that elaboration lowers into other Core IR: `Map.update`;
//! - **the syntax tree**, for imports of the standard modules and names an entry adds, and the
//!   header clause `diverges`, which Core IR does not keep;
//! - **the tokens**, for the words an edition frees or gives a new place: `simd` as a name, and
//!   the argument labels `with:` and `and:`;
//! - **the project files**, for `ita.una`, `ita.lock`, the `needs`, `sources` and `trust`
//!   lines, grants with limits, `serve:` grants, and the `drive` and `serve` sections.
//!
//! Only the root package's modules count: a dependency is reported under `dependencies`.
//! The walk visits each node of the root's Core IR, syntax tree and tokens once: O(n) in the
//! size of the package, and O(k) memory per site for k entries. Sorting the entries is
//! O(k log k).

use std::collections::BTreeSet;

use una_core::{Base, Bound, Decl, Expr, LoopSource, Prim, Rhs, Source, Type};
use una_project::ledger::{self, Construct, Entry, LEDGER};
use una_syntax::ast::{ClauseKind, DeclKind};
use una_syntax::{FileId, Interner, Keyword, Span, TokenKind};

use crate::Analysis;
use crate::package::Package;

/// A site of a construct: a span of the root's source, or, for what Core IR and the project
/// files keep without one, a name of its place.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Site {
    /// A range of a file.
    Span(FileId, usize, usize),
    /// A declaration, a test or a line of a project file, by name.
    Named(String),
}

impl From<Span> for Site {
    fn from(span: Span) -> Site {
        Site::Span(span.file, span.start, span.end)
    }
}

/// The sites of each construct, by its index in the ledger.
struct Sites(Vec<BTreeSet<Site>>);

impl Sites {
    fn add(&mut self, construct: Construct, site: impl Into<Site>) {
        if let Some(sites) = self.0.get_mut(construct as usize) {
            sites.insert(site.into());
        }
    }
}

/// Where the report counts each construct: the match is exhaustive, so a construct added to
/// the ledger without a counter does not compile, and the report's test of every entry then
/// asks for a package that uses it.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn counted_in(construct: Construct) -> &'static str {
    match construct {
        Construct::ForUntil
        | Construct::StopSkip
        | Construct::ForEver
        | Construct::ForEverWithin
        | Construct::StopAfter
        | Construct::Within
        | Construct::FirstOf
        | Construct::TasksStart
        | Construct::Sum
        | Construct::Dot
        | Construct::MatrixKernels
        | Construct::Dense
        | Construct::Decimal
        | Construct::F32
        | Construct::Complex => "Core IR",
        Construct::Limiter
        | Construct::Collections
        | Construct::StdTime
        | Construct::HttpServer
        | Construct::Drive => "Core IR, imports and project files",
        Construct::MapUpdate => "the checker",
        Construct::Diverges | Construct::ModuleConstants => "the syntax tree",
        Construct::SimdName | Construct::ArgumentLabels | Construct::ConstVar => "the tokens",
        Construct::GrantLimits
        | Construct::ItaManifest
        | Construct::ItaLock
        | Construct::Needs
        | Construct::Serve => "the project files",
    }
}

/// The constructs of the ledger that `package` uses, each with its number of sites, ordered
/// by record, then construct: `[]` for a package of a stable edition or of edition `0.3`,
/// which has no ledger entry (ADR 0121 rule 13).
pub(crate) fn preview_constructs(
    package: &Package,
    analysis: &Analysis,
    manifest: &una_project::Manifest,
) -> Vec<(&'static Entry, usize)> {
    let edition = manifest.edition;
    if ledger::in_edition(edition).next().is_none() {
        return Vec::new();
    }
    let mut sites = Sites(vec![BTreeSet::new(); Construct::COUNT]);
    // The root's own module files: those of no mounted dependency.
    let root: BTreeSet<FileId> = package
        .modules
        .iter()
        .copied()
        .filter(|id| {
            manifest
                .mount_of(package.sources.file(*id).path())
                .is_none()
        })
        .collect();
    let mut modules: BTreeSet<String> = BTreeSet::new();
    for (id, file) in analysis.trees.iter().filter(|(id, _)| root.contains(id)) {
        if let Some(header) = &file.module {
            let path: Vec<&str> = header
                .path
                .segments
                .iter()
                .map(|segment| analysis.interner.resolve(segment.name))
                .collect();
            modules.insert(path.join("."));
        }
        syntax(file, &analysis.interner, &mut sites);
        tokens(package, *id, edition, &mut sites);
    }
    for module in analysis
        .program
        .modules
        .iter()
        .filter(|module| modules.contains(&module.path))
    {
        core_module(module, &mut sites);
    }
    for (construct, span) in &analysis.preview_sites {
        if root.contains(&span.file) {
            sites.add(*construct, *span);
        }
    }
    project_files(package, manifest, &mut sites);
    let mut used: Vec<(&'static Entry, usize)> = ledger::in_edition(edition)
        .filter_map(|entry| {
            let count = sites.0.get(entry.id as usize).map_or(0, BTreeSet::len);
            (count > 0).then_some((entry, count))
        })
        .collect();
    used.sort_by(|a, b| (a.0.record, a.0.name).cmp(&(b.0.record, b.0.name)));
    used
}

/// The entries that a standard declaration `name` of `module` belongs to: those that list
/// `module.name`, or else those that list the module itself.
fn standard(module: &str, name: Option<&str>) -> Vec<Construct> {
    let qualified = name.map(|name| format!("{module}.{name}"));
    let named: Vec<Construct> = LEDGER
        .iter()
        .filter(|entry| {
            qualified
                .as_deref()
                .is_some_and(|q| entry.modules.contains(&q))
        })
        .map(|entry| entry.id)
        .collect();
    if !named.is_empty() {
        return named;
    }
    LEDGER
        .iter()
        .filter(|entry| entry.modules.contains(&module))
        .map(|entry| entry.id)
        .collect()
}

/// The imports and the header clauses of a root module.
fn syntax(file: &una_syntax::ast::File, interner: &Interner, sites: &mut Sites) {
    for import in &file.imports {
        let module: Vec<&str> = import
            .module
            .segments
            .iter()
            .map(|segment| interner.resolve(segment.name))
            .collect();
        let module = module.join(".");
        if import.names.is_empty() {
            for construct in standard(&module, None) {
                sites.add(construct, import.span);
            }
        }
        for name in &import.names {
            for construct in standard(&module, Some(interner.resolve(name.name))) {
                sites.add(construct, name.span);
            }
        }
    }
    for decl in &file.decls {
        // ADR 0126: each module constant.
        if let DeclKind::Const(constant) = &decl.kind {
            sites.add(Construct::ModuleConstants, constant.name.span);
        }
        if let DeclKind::Function(function) = &decl.kind
            && function
                .clauses
                .iter()
                .any(|clause| matches!(clause.kind, ClauseKind::Diverges))
        {
            sites.add(Construct::Diverges, decl.span);
        }
    }
}

/// The words of a root module: `simd` as a name, `with:` and `and:` as labels, and the
/// binding words `const` and `var`.
fn tokens(package: &Package, id: FileId, edition: una_project::Edition, sites: &mut Sites) {
    let mut interner = Interner::new();
    let lexed = una_syntax::lex_in(
        package.sources.file(id),
        id,
        &mut interner,
        crate::keyword_set(edition),
    );
    let simd = interner.intern("simd");
    let mut tokens = lexed.tokens.iter().peekable();
    while let Some(token) = tokens.next() {
        match token.kind {
            TokenKind::LowerIdent(word)
                if word == simd && edition.has_collections_and_numerics() =>
            {
                sites.add(Construct::SimdName, token.span);
            }
            // ADR 0137: each binding written `const` or `var`.
            TokenKind::Keyword(Keyword::Const | Keyword::Var) => {
                sites.add(Construct::ConstVar, token.span);
            }
            TokenKind::Keyword(Keyword::With | Keyword::And)
                if tokens
                    .peek()
                    .is_some_and(|next| matches!(next.kind, TokenKind::Colon)) =>
            {
                sites.add(Construct::ArgumentLabels, token.span);
            }
            _ => {}
        }
    }
}

/// The project files: `ita.una`, `ita.lock`, and the sections and grants that entries add.
fn project_files(package: &Package, manifest: &una_project::Manifest, sites: &mut Sites) {
    let Some(ita) = manifest.ita.as_deref() else {
        return;
    };
    let at = |range: &std::ops::Range<usize>| match package.manifest {
        Some(file) => Site::Span(file, range.start, range.end),
        None => Site::Named(format!("ita.una {}..{}", range.start, range.end)),
    };
    sites.add(Construct::ItaManifest, Site::Named("ita.una".to_owned()));
    if package.lock.is_some() {
        sites.add(Construct::ItaLock, Site::Named("ita.lock".to_owned()));
    }
    for (index, _) in ita.needs.iter().enumerate() {
        sites.add(Construct::Needs, Site::Named(format!("need {index}")));
    }
    for (index, _) in ita.sources.iter().enumerate() {
        sites.add(Construct::Needs, Site::Named(format!("source {index}")));
    }
    for (index, _) in ita.trust.iter().enumerate() {
        sites.add(Construct::Needs, Site::Named(format!("trust {index}")));
    }
    if let Some(run) = &ita.run {
        for grant in &run.grants {
            if !grant.limits.is_empty() {
                sites.add(Construct::GrantLimits, at(&grant.range));
            }
            if grant.spec.starts_with("serve:") {
                sites.add(Construct::HttpServer, at(&grant.range));
            }
        }
    }
    if let Some(drive) = &ita.drive {
        sites.add(Construct::Drive, at(&drive.range));
    }
    if ita.serve.is_some() {
        sites.add(Construct::Serve, Site::Named("serve".to_owned()));
    }
}

/// The Core IR of a root module.
fn core_module(module: &una_core::Module, sites: &mut Sites) {
    let mut walk = Walk { sites };
    for decl in &module.decls {
        let place = |name: &str| Site::Named(format!("{}.{name}", module.path));
        match decl {
            Decl::Fun(fun) => {
                let at = Site::from(fun.span);
                for (_, ty) in &fun.params {
                    walk.ty(ty, &at);
                }
                walk.ty(&fun.result, &at);
                walk.expr(&fun.body, &at);
            }
            Decl::Pred {
                params, body, span, ..
            } => {
                let at = Site::from(*span);
                for (_, ty) in params {
                    walk.ty(ty, &at);
                }
                walk.expr(body, &at);
            }
            Decl::Data { name, variants, .. } => {
                let at = place(name);
                for variant in variants {
                    for (_, ty) in &variant.fields {
                        walk.ty(ty, &at);
                    }
                }
            }
            Decl::Alias { name, target, .. } => walk.ty(target, &place(name)),
            Decl::Cap { name, ops } => {
                let at = place(name);
                for op in ops {
                    for (_, ty) in &op.params {
                        walk.ty(ty, &at);
                    }
                    walk.ty(&op.result, &at);
                }
            }
            Decl::Handler {
                name, params, ops, ..
            } => {
                let at = place(name);
                for (_, ty) in params {
                    walk.ty(ty, &at);
                }
                for op in ops {
                    walk.expr(&op.body, &at);
                }
            }
            Decl::Policy { name, from, to, .. } => {
                let at = place(name);
                walk.ty(from, &at);
                walk.ty(to, &at);
            }
        }
    }
    for example in &module.examples {
        let at = Site::Named(format!("{} test {:?}", module.path, example.name));
        if example.stop_after.is_some() {
            walk.sites.add(Construct::StopAfter, at.clone());
        }
        walk.expr(&example.body, &at);
    }
}

/// The receivers of the dense kernels, as the prelude names their functions
/// (`dense_<receiver>_<method>`), and as the ledger names them.
const DENSE_RECEIVERS: [(&str, &str); 4] = [
    ("array_", "Array"),
    ("matrix_", "Matrix"),
    ("tensor_", "Tensor"),
    ("lu_", "Lu"),
];

struct Walk<'a> {
    sites: &'a mut Sites,
}

impl Walk<'_> {
    /// The constructs of the types an entry adds that `ty` holds, at `at`.
    fn ty(&mut self, ty: &Type, at: &Site) {
        match ty {
            Type::Base { base, .. } => self.base(base, at),
            Type::Fn { params, result, .. } => {
                for param in params {
                    self.ty(param, at);
                }
                self.ty(result, at);
            }
        }
    }

    fn base(&mut self, base: &Base, at: &Site) {
        match base {
            Base::Decimal(_) => self.sites.add(Construct::Decimal, at.clone()),
            Base::F32 => self.sites.add(Construct::F32, at.clone()),
            Base::ReplyRight => self.sites.add(Construct::HttpServer, at.clone()),
            Base::Named { module, name, args } => {
                self.named(module, name, at);
                for arg in args {
                    self.ty(arg, at);
                }
            }
            Base::Tuple(items) => {
                for item in items {
                    self.ty(item, at);
                }
            }
            _ => {}
        }
    }

    /// A kind or function `name` of `module` that Core IR names.
    fn named(&mut self, module: &str, name: &str, at: &Site) {
        if una_core::complex::is_kind(module, name) {
            self.sites.add(Construct::Complex, at.clone());
        } else if module == una_core::COLLECTIONS_MODULE {
            self.sites.add(Construct::Collections, at.clone());
        } else if module == una_core::PRELUDE_MODULE {
            for entry in LEDGER.iter().filter(|entry| entry.prelude.contains(&name)) {
                self.sites.add(entry.id, at.clone());
            }
        } else {
            for construct in standard(module, Some(name)) {
                self.sites.add(construct, at.clone());
            }
        }
    }

    fn expr(&mut self, expr: &Expr, at: &Site) {
        match expr {
            Expr::Atom { .. } | Expr::Unreachable { .. } => {}
            Expr::Let {
                ty,
                value,
                body,
                span,
                ..
            } => {
                let here = Site::from(*span);
                self.ty(ty, &here);
                match value.as_ref() {
                    Bound::Rhs(rhs) => self.rhs(rhs, &here),
                    Bound::Expr(inner) => self.expr(inner, &here),
                }
                self.expr(body, at);
            }
            Expr::LetTuple {
                names, body, span, ..
            } => {
                let here = Site::from(*span);
                for (_, ty) in names {
                    self.ty(ty, &here);
                }
                self.expr(body, at);
            }
            Expr::Loop {
                source,
                var,
                state,
                body,
                on_stop,
                span,
                ..
            } => {
                let here = Site::from(*span);
                self.source(source, &here);
                self.ty(&var.1, &here);
                for (_, ty) in state {
                    self.ty(ty, &here);
                }
                self.expr(body, &here);
                if let Some(on_stop) = on_stop {
                    self.expr(&on_stop.body, &here);
                }
            }
            Expr::Exit { span, .. } => self.sites.add(Construct::StopSkip, *span),
            Expr::Together { tasks, .. } => {
                for task in tasks {
                    self.expr(task, at);
                }
            }
            Expr::TogetherFor {
                source,
                var,
                task,
                span,
            } => {
                let here = Site::from(*span);
                self.source(source, &here);
                self.ty(&var.1, &here);
                self.expr(task, &here);
            }
            Expr::FirstOf { tasks, span } => {
                self.sites.add(Construct::FirstOf, *span);
                for task in tasks {
                    self.expr(task, &Site::from(*span));
                }
            }
            Expr::Tasks { body, span, .. } => {
                self.sites.add(Construct::TasksStart, *span);
                self.expr(body, &Site::from(*span));
            }
            Expr::Start { task, span, .. } => {
                self.sites.add(Construct::TasksStart, *span);
                self.expr(task, &Site::from(*span));
            }
            Expr::Quiet { body, .. } | Expr::Attempt { body, .. } => self.expr(body, at),
            Expr::If {
                then, otherwise, ..
            } => {
                self.expr(then, at);
                self.expr(otherwise, at);
            }
            Expr::Case {
                arms, otherwise, ..
            } => {
                for arm in arms {
                    self.variant(&arm.ctor, at);
                    self.expr(&arm.body, at);
                }
                if let Some(otherwise) = otherwise {
                    self.expr(otherwise, at);
                }
            }
            Expr::Fail { ctor, .. } => self.variant(ctor, at),
            Expr::Handle { body, handler, .. } => {
                self.expr(body, at);
                self.expr(handler, at);
            }
            Expr::Within { body, span, .. } => {
                self.sites.add(Construct::Within, *span);
                self.expr(body, &Site::from(*span));
            }
            Expr::With { source, body, .. } => {
                if let Source::Handler { handler, span, .. } = source {
                    if handler.module == una_core::LIMIT_MODULE && handler.name == una_core::LIMITER
                    {
                        self.sites.add(Construct::Limiter, *span);
                    }
                    self.expr(body, &Site::from(*span));
                } else {
                    self.expr(body, at);
                }
            }
        }
    }

    /// What a loop visits: `for until` and `for ever`.
    fn source(&mut self, source: &LoopSource, at: &Site) {
        match source {
            LoopSource::Until { cond, .. } => {
                self.sites.add(Construct::ForUntil, at.clone());
                self.expr(cond, at);
            }
            LoopSource::Ever { within: None } => self.sites.add(Construct::ForEver, at.clone()),
            LoopSource::Ever { within: Some(_) } => {
                self.sites.add(Construct::ForEverWithin, at.clone());
            }
            LoopSource::List(_) | LoopSource::Range { .. } => {}
        }
    }

    /// A variant that a pattern or a `fail` names: those an entry adds to an existing kind.
    fn variant(&mut self, ctor: &una_core::Ctor, at: &Site) {
        for entry in &LEDGER {
            if entry
                .variants
                .iter()
                .any(|(kind, variant)| *kind == ctor.kind.name && *variant == ctor.name)
            {
                self.sites.add(entry.id, at.clone());
            }
        }
        self.named(&ctor.kind.module, &ctor.kind.name, at);
    }

    fn rhs(&mut self, rhs: &Rhs, at: &Site) {
        match rhs {
            Rhs::Prim { prim, span, .. } => {
                let construct = match prim {
                    Prim::Decimal(_) => Some(Construct::Decimal),
                    Prim::Float32(_) => Some(Construct::F32),
                    Prim::Seq(_)
                    | Prim::Set(_)
                    | Prim::HashMap(_)
                    | Prim::HashSet(_)
                    | Prim::Priority(_)
                    | Prim::BitSet(_) => Some(Construct::Collections),
                    _ => None,
                };
                if let Some(construct) = construct {
                    self.sites.add(construct, *span);
                }
            }
            Rhs::Call {
                function,
                type_args,
                span,
                ..
            } => {
                let here = Site::from(*span);
                if function.module == una_core::PRELUDE_MODULE
                    && let Some(rest) = function.name.strip_prefix("dense_")
                {
                    self.kernel(rest, &here);
                }
                self.named(&function.module, &function.name, &here);
                for ty in type_args {
                    self.ty(ty, &here);
                }
            }
            Rhs::Op {
                capability_path,
                span,
                ..
            } => self.named(
                &capability_path.module,
                &capability_path.name,
                &Site::from(*span),
            ),
            Rhs::Construct { ctor, .. } => self.variant(ctor, at),
            Rhs::Lambda { params, body, .. } => {
                for (_, ty) in params {
                    self.ty(ty, at);
                }
                self.expr(body, at);
            }
            Rhs::Checked { predicate, .. } => self.expr(predicate, at),
            Rhs::Atom(_)
            | Rhs::Tuple(_)
            | Rhs::Apply { .. }
            | Rhs::Field { .. }
            | Rhs::List(_)
            | Rhs::Declassify { .. } => {}
        }
    }

    /// A dense kernel, `dense_<receiver>_<method>` of the prelude: the entries whose built-in
    /// methods list it, `sum(axis:)` as `sum`.
    fn kernel(&mut self, rest: &str, at: &Site) {
        self.sites.add(Construct::Dense, at.clone());
        let Some((receiver, method)) = DENSE_RECEIVERS
            .iter()
            .find_map(|(prefix, receiver)| rest.strip_prefix(prefix).map(|m| (*receiver, m)))
        else {
            return;
        };
        let method = if method == "sum_axis" { "sum" } else { method };
        for entry in &LEDGER {
            if entry
                .methods
                .iter()
                .any(|(r, m)| *r == receiver && *m == method)
            {
                self.sites.add(entry.id, at.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_construct_has_a_counter() {
        for entry in &LEDGER {
            assert!(!counted_in(entry.id).is_empty(), "{}", entry.name);
        }
    }
}
