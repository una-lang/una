//! The index of a checked package, for the language server (ADR 0063): what each resolved
//! name refers to, the type of each expression, and the function each call names.

use una_syntax::Span;
use una_syntax::ast::{DeclKind, KindBody};

use super::Findings;
use super::env::{Env, FnAst};
use super::records::Res;
use super::ty::{KindId, Names, ShowLabeled, Ty};

/// What the type checker resolved, by span, for an editor.
#[derive(Clone, Debug, Default)]
pub struct Index {
    /// Each call, name, constructor, operation and installed handler that names a
    /// declaration of the package: its span, and the span of the declaration's name.
    pub definitions: Vec<(Span, Span)>,
    /// Each expression and binder: its span, and its type as written, labels included.
    pub types: Vec<(Span, String)>,
    /// Each call of, or reference to, a function of the package: its span, the function's
    /// module path and its name.
    pub functions: Vec<(Span, String, String)>,
}

/// The names of types, for one declaration: the kinds of the package and its type
/// parameters.
struct DeclNames<'e, 'a> {
    env: &'e Env<'a>,
    params: Vec<String>,
}

impl Names for DeclNames<'_, '_> {
    fn kind_name(&self, id: KindId) -> String {
        self.env.kind_name(id).to_owned()
    }

    fn param_name(&self, index: usize) -> String {
        self.params
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("T{index}"))
    }
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
    // The type parameters of each declaration of the package, by its span.
    let mut scopes: Vec<(Span, Vec<String>)> = Vec::new();
    for module in env.modules.iter().take(env.prelude) {
        for decl in &module.tree.decls {
            if let DeclKind::Function(function) = &decl.kind {
                let params = function
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
    let params_at = |span: Span| -> Vec<String> {
        let after =
            scopes.partition_point(|(decl, _)| (decl.file, decl.start) <= (span.file, span.start));
        after
            .checked_sub(1)
            .and_then(|last| scopes.get(last))
            .filter(|(decl, _)| {
                decl.file == span.file && decl.start <= span.start && span.end <= decl.end
            })
            .map(|(_, params)| params.clone())
            .unwrap_or_default()
    };
    let records = &findings.records;
    for (span, res) in &records.res {
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
            Res::Value | Res::Len | Res::IntMethod(_) | Res::MapMethod(_) | Res::MapUpdate(_) => {
                None
            }
        };
        // Only declarations of the package have a place to go to; the standard modules are
        // left out above, since their sources are another database.
        if let Some(target) = target {
            index.definitions.push((*span, target));
        }
    }
    for (span, ty) in &records.types {
        if matches!(ty, Ty::Error) {
            continue;
        }
        let names = DeclNames {
            env,
            params: params_at(*span),
        };
        index
            .types
            .push((*span, ShowLabeled(ty, &names).to_string()));
    }
    index
}
