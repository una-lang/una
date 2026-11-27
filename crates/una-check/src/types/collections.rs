//! The names of the collections of edition 0.4 (ADR 0119 rules 0 and 18): a package function
//! named like a built-in method of a collection, which method syntax could never reach.

use std::collections::BTreeMap;

use una_core::Coll;
use una_diag::Code;
use una_project::Edition;
use una_syntax::Span;

use super::Report;
use super::env::{Entity, Env, FnAst};
use super::ty::Ty;

/// The built-in methods of each collection type (ADR 0119 rules 5 to 7 and 12 to 14).
pub(crate) fn methods(coll: Coll) -> &'static [&'static str] {
    match coll {
        Coll::Stack => &["push", "pop", "peek", "items"],
        Coll::Queue => &[
            "push",
            "pop",
            "peek",
            "push_front",
            "pop_back",
            "peek_back",
            "items",
        ],
        Coll::Set => &[
            "has",
            "add",
            "remove",
            "with",
            "without",
            "first",
            "last",
            "between",
            "items",
            "union",
            "intersection",
            "difference",
            "is_subset",
        ],
        Coll::HashMap => &[
            "has",
            "get",
            "set",
            "remove",
            "with",
            "without",
            "update",
            "keys",
            "values",
            "entries",
            "same_entries",
        ],
        Coll::HashSet => &[
            "has",
            "add",
            "remove",
            "with",
            "without",
            "items",
            "union",
            "intersection",
            "difference",
            "is_subset",
            "same_items",
        ],
        Coll::PriorityQueue => &[
            "push",
            "pop",
            "pop_ranked",
            "peek",
            "peek_priority",
            "items",
        ],
        Coll::BitSet => &[
            "has",
            "add",
            "remove",
            "with",
            "without",
            "union",
            "intersection",
            "difference",
            "is_subset",
            "first",
            "last",
            "items",
        ],
    }
}

/// Whether `.len` of the collection type `coll` resolves with the labels of its shape (ADR
/// 0119 rules 5 to 7 and 12 to 14): each type gains it with its methods. `Stack` and `Queue`
/// resolve it before, with their top label (IF1).
pub(crate) fn has_len(coll: Coll) -> bool {
    match coll {
        Coll::Set | Coll::HashMap | Coll::HashSet | Coll::PriorityQueue | Coll::BitSet => true,
        Coll::Stack | Coll::Queue => false,
    }
}

/// The types whose built-in method a package function named `name`, whose first parameter is
/// of type `first`, would hide from method syntax in edition 0.4, in the order of ADR 0119's
/// export list, with `Map` first for `update` (rule 18). Empty when it hides none.
fn hidden(name: &str, first: &Ty) -> Vec<&'static str> {
    let mut types = Vec::new();
    match first.unlabeled() {
        Ty::Coll(coll, _) if methods(*coll).contains(&name) => types.push(coll.as_str()),
        Ty::Map(..) if name == "update" => types.push("Map"),
        Ty::Param(_) => {
            if name == "update" {
                types.push("Map");
            }
            types.extend(
                Coll::ALL
                    .into_iter()
                    .filter(|coll| methods(*coll).contains(&name))
                    .map(Coll::as_str),
            );
        }
        _ => {}
    }
    types
}

/// The types listed as a message names them: "`Queue`", "`Map` and `HashMap`".
fn listed(types: &[&str]) -> String {
    let quoted: Vec<String> = types.iter().map(|name| format!("`{name}`")).collect();
    match quoted.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// U1002, edition 0.4: each package function whose name is a built-in method of a collection
/// type and whose first parameter has that type, refined or not, or is a type parameter, at its
/// name (ADR 0119 rule 0), and each `update` whose first parameter is a `Map` (rule 18); then
/// each import of such a function, at the imported name. A built-in method resolves ahead of a
/// function of the same name (`spec/04 §3`), so method syntax would never reach it. Linear in
/// the functions and the imports. Each rule is the edition's of the module that declares or
/// imports the function (ADR 0122 rule 49): a dependency of edition 0.4 may declare `update` on
/// a `Map`, and a module of edition 0.5 that imports it is U1002 at the import.
pub(crate) fn method_names(env: &Env<'_>, report: &mut Report<'_>) {
    let hidden_of = |def: &super::env::FnDef<'_>| {
        def.params
            .first()
            .map_or_else(Vec::new, |first| hidden(def.name, &first.ty.base))
    };
    let mut flagged: BTreeMap<usize, Vec<&'static str>> = BTreeMap::new();
    for (id, def) in env.functions.iter().enumerate() {
        if !env.in_package(def.module) || !env.reports(def.module) {
            continue;
        }
        let types = hidden_of(def);
        if types.is_empty() {
            continue;
        }
        // A function declared in a module of an earlier edition is not U1002 there, and its
        // imports into a module of edition 0.5 are.
        if env.edition_of(def.module) < Edition::V0_5 {
            flagged.insert(id, types);
            continue;
        }
        let span = name_span(def.ast);
        report.error(
            Code::U1002,
            span,
            format!(
                "`{}` is a built-in method of {}; rename this function",
                def.name,
                listed(&types)
            ),
        );
        flagged.insert(id, types);
    }
    if flagged.is_empty() {
        return;
    }
    for (index, module) in env.modules.iter().enumerate().take(env.prelude) {
        if !env.reports(index) || module.edition < Edition::V0_5 {
            continue;
        }
        for import in &module.tree.imports {
            for ident in &import.names {
                let name = module.interner.resolve(ident.name);
                let Some(Entity::Function(id)) = module.imports.get(name) else {
                    continue;
                };
                let Some(types) = flagged.get(id) else {
                    continue;
                };
                report.error(
                    Code::U1002,
                    ident.span,
                    format!(
                        "`{name}` is a built-in method of {}, so method syntax never reaches the function this imports; rename it",
                        listed(types)
                    ),
                );
            }
        }
    }
}

/// Where a function, predicate or policy declares its name.
fn name_span(ast: FnAst<'_>) -> Span {
    match ast {
        FnAst::Function(decl) => decl.name.span,
        FnAst::Predicate(decl) => decl.name.span,
        FnAst::Policy(decl) => decl.name.span,
    }
}
