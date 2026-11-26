//! The index of a checked package, for the language server (ADR 0063): what each resolved
//! name refers to, the type of each expression, and the function each call names.

use std::sync::Arc;

use una_syntax::Span;
use una_syntax::ast::{DeclKind, KindBody};

use super::Findings;
use super::env::{Env, FnAst};
use super::hover::{Hover, HoverNames, HoverStats, Snapshots};
use super::records::Res;
use super::ty::{KindId, Ty};

/// What the type checker resolved, by span, for an editor.
#[derive(Clone, Debug, Default)]
pub struct Index {
    /// Each call, name, constructor, operation and installed handler that names a
    /// declaration of the package: its span, and the span of the declaration's name.
    pub definitions: Vec<(Span, Span)>,
    /// Each expression and binder: its span, and its type, printed as written, labels
    /// included, when the editor asks for it (ADR 0080).
    pub types: Vec<(Span, Hover)>,
    /// What building the hovers did, for the scaling test (ADR 0080).
    pub hover_stats: HoverStats,
    /// Each call of, or reference to, a function of the package: its span, the function's
    /// module path and its name.
    pub functions: Vec<(Span, String, String)>,
    /// Each reference to a module constant (ADR 0126 rule 11): its span, and the hover text
    /// that shows the constant's declared type and, for a scalar, its value.
    pub constants: Vec<(Span, String)>,
}

/// The span of the name that declares the kind `kind`, or its variant `variant`, when a
/// module of the package declares it.
fn kind_span(env: &Env<'_>, kind: KindId, variant: Option<usize>) -> Option<Span> {
    let def = env.kinds.get(kind).filter(|def| def.module < env.prelude)?;
    match (variant, &def.ast.body) {
        (Some(index), KindBody::Sum(variants)) => variants.get(index).map(|v| v.name.span),
        _ => Some(def.ast.name.span),
    }
}

/// The index of the package that `env` and `findings` describe.
pub(crate) fn build(env: &Env<'_>, findings: &Findings) -> Index {
    let mut index = Index::default();
    // The type parameters of each declaration of the package, by its span, shared by the
    // hovers of its expressions.
    let mut scopes: Vec<(Span, Arc<[String]>)> = Vec::new();
    for module in env.modules.iter().take(env.prelude) {
        for decl in &module.tree.decls {
            if let DeclKind::Function(function) = &decl.kind {
                let params: Arc<[String]> = function
                    .type_params
                    .iter()
                    .map(|p| module.interner.resolve(p.name).to_owned())
                    .collect();
                scopes.push((decl.span, params));
            }
        }
    }
    // Top-level declarations of one file do not overlap, so the one that contains a span is
    // the last that starts at or before it: a binary search, O(log n) per span (#375).
    scopes.sort_by_key(|(decl, _)| (decl.file, decl.start));
    let none: Arc<[String]> = Arc::from(Vec::new());
    let params_at = |span: Span| -> Arc<[String]> {
        let after =
            scopes.partition_point(|(decl, _)| (decl.file, decl.start) <= (span.file, span.start));
        after
            .checked_sub(1)
            .and_then(|last| scopes.get(last))
            .filter(|(decl, _)| {
                decl.file == span.file && decl.start <= span.start && span.end <= decl.end
            })
            .map_or_else(|| Arc::clone(&none), |(_, params)| Arc::clone(params))
    };
    let records = &findings.records;
    for (span, res) in &records.res {
        // ADR 0126 rule 11: a constant's hover shows its type and value.
        if let Res::Const(id) = res
            && let Some(def) = env.constants.get(*id)
        {
            let evaluated = if def.module >= env.prelude {
                &findings.standard_constants
            } else {
                &findings.constants
            };
            let value = evaluated
                .folded
                .get(id)
                .and_then(|folded| folded.get(&def.ast.value.span))
                .map(una_core::print_atom);
            let ty = env.show_plain(&env.top_refined(&def.declared));
            let text = match value {
                Some(value) => format!("```una\nconst {}: {ty} = {value}\n```", def.name),
                None => format!("```una\nconst {}: {ty}\n```", def.name),
            };
            index.constants.push((*span, text));
        }
        let target = match res {
            Res::Function { id, .. } => {
                let Some(def) = env.functions.get(*id) else {
                    continue;
                };
                if def.module >= env.prelude {
                    continue;
                }
                if let Some(module) = env.modules.get(def.module) {
                    index
                        .functions
                        .push((*span, module.path.clone(), def.name.to_owned()));
                }
                match def.ast {
                    FnAst::Function(function) => Some(function.name.span),
                    FnAst::Predicate(predicate) => Some(predicate.name.span),
                    FnAst::Policy(policy) => Some(policy.name.span),
                }
            }
            Res::Ctor { kind, variant } => kind_span(env, *kind, *variant),
            Res::Field { kind, .. } | Res::FieldFn { kind, .. } => kind_span(env, *kind, None),
            Res::Op { capability } => env
                .capabilities
                .get(*capability)
                .filter(|c| c.module < env.prelude)
                .map(|c| c.ast.name.span),
            Res::Handler { id } => env
                .handlers
                .get(*id)
                .filter(|h| h.module < env.prelude)
                .map(|h| h.ast.name.span),
            // A module constant goes to its declaration (ADR 0126 rule 11).
            Res::Const(id) => env
                .constants
                .get(*id)
                .filter(|c| c.module < env.prelude)
                .map(|c| c.ast.name.span),
            Res::Value
            | Res::Len
            | Res::IntMethod(_)
            | Res::UnitMethod { .. }
            | Res::Convert { .. }
            | Res::MapMethod(_)
            | Res::MapUpdate(_)
            | Res::SeqMethod(_)
            | Res::SeqUpdate(_)
            | Res::SetMethod(_)
            | Res::SetUpdate(_)
            | Res::HashMapMethod(_)
            | Res::HashMapUpdate(_)
            | Res::HashSetMethod(_)
            | Res::HashSetUpdate(_)
            | Res::PriorityMethod(_)
            | Res::PriorityUpdate { .. }
            | Res::BitSetMethod(_)
            | Res::BitSetUpdate(_)
            | Res::Constant(_)
            | Res::Shape(_)
            | Res::DenseElement(_)
            | Res::TensorElement(_)
            | Res::Dim(_)
            | Res::Complex(_)
            | Res::DenseOp { .. } => None,
        };
        // Only declarations of the package have a place to go to; the standard modules are
        // left out above, since their sources are another database.
        if let Some(target) = target {
            index.definitions.push((*span, target));
        }
    }
    // ADR 0080: each type is kept as a `Send` copy, made once per shared node of the checker's
    // types, and printed only when a hover asks for it; kinds are named as declared.
    let names = Arc::new(HoverNames {
        kinds: (0..env.kinds.len())
            .map(|id: KindId| {
                // A complex kind is written as its type, `Complex[F64]` (ADR 0120 rule 22).
                env.complex_written(id)
                    .map_or_else(|| env.kind_name(id).to_owned(), ToOwned::to_owned)
            })
            .collect(),
    });
    let mut snapshots = Snapshots::default();
    for (span, ty) in &records.types {
        if matches!(ty, Ty::Error(_)) {
            continue;
        }
        let hover = Hover::new(snapshots.of(ty), params_at(*span), Arc::clone(&names));
        index.types.push((*span, hover));
    }
    index.hover_stats = snapshots.stats;
    index
}
