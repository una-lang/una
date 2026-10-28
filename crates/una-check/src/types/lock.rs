//! Edition 0.3: the locked contracts of `una.toml` (ADR 0092 rule 8, as amended).
//!
//! [`resolve`] reads each lock of `[contracts]` against the package: its function, and for each
//! of its `uses` clauses the binding, the capability by the declaration its name resolves to,
//! and the operations. A lock that does not resolve is U7002, never a lock that checks
//! nothing. [`drift`] then reports each binding and operation of a locked header that its lock
//! does not grant (U3023). The conditions are checked with the function's header, and their
//! implications are `contract_lock` obligations. One pass over the locks and one over the
//! locked headers, with maps keyed by function: linear in their size.

use std::collections::BTreeMap;

use una_diag::{Code, Related};
use una_syntax::Span;
use una_syntax::ast::{ClauseKind, UsesClause};

use super::Report;
use super::env::{Entity, Env, FnAst, FnKind};
use super::ty::CapId;
use crate::names::{Lock, LockClause};

/// A well-formed locked clause of a function.
#[derive(Clone, Debug)]
pub(crate) struct Entry<'a> {
    /// The clause, as `una.toml` writes it.
    pub(crate) clause: &'a UsesClause,
    /// Its binding, without `@`.
    pub(crate) binding: &'a str,
    /// The capability its name resolves to in the function's module.
    pub(crate) capability: CapId,
    /// The operations it lists.
    pub(crate) ops: Vec<&'a str>,
    /// Where its string is written in `una.toml`.
    pub(crate) span: Span,
    /// The locked clause, for the text of its parts as its string reads (#727).
    pub(crate) source: &'a LockClause,
}

/// Resolves each lock of `locks` into `env.locks`, reporting U7002 for each part that does not
/// resolve.
pub(crate) fn resolve<'a>(env: &mut Env<'a>, locks: &'a [Lock], report: &mut Report<'_>) {
    if locks.is_empty() {
        return;
    }
    let code = env.project_code;
    // The package's modules by path, so that each key is found in O(log m).
    let modules: BTreeMap<String, usize> = (0..env.prelude)
        .filter_map(|m| env.modules.get(m).map(|module| (module.path.clone(), m)))
        .collect();
    for lock in locks {
        let Some(id) = function(env, &modules, &lock.function) else {
            report.error(
                code,
                lock.key,
                format!(
                    "[contracts] locks `{}`, which is not a public function of this package",
                    lock.function
                ),
            );
            continue;
        };
        let Some((module, name)) = env.functions.get(id).map(|def| (def.module, def.name)) else {
            continue;
        };
        let mut entries: Vec<Entry<'a>> = Vec::new();
        for clause in &lock.clauses {
            let parsed = match &clause.parsed {
                Ok(parsed) => parsed,
                Err(error) => {
                    report.error(
                        code,
                        clause.span,
                        format!(
                            "the lock of `{name}` has an entry that is not a `uses` clause: {}",
                            error.message
                        ),
                    );
                    continue;
                }
            };
            let binding = env.text(module, parsed.cap.name);
            if parsed.cap_type.ops.is_empty() {
                report.error(
                    code,
                    clause.span,
                    format!(
                        "the lock of `{name}` writes `@{binding}` without its operations; list them in brackets"
                    ),
                );
                continue;
            }
            if entries.iter().any(|entry| entry.binding == binding) {
                report.error(
                    code,
                    clause.span,
                    format!("the lock of `{name}` locks `@{binding}` twice"),
                );
                continue;
            }
            let cap_name = env.text(module, parsed.cap_type.name.name);
            let Some(Entity::Capability(capability)) = env.upper(module, cap_name).first().copied()
            else {
                report.error(
                    code,
                    clause.span,
                    format!(
                        "the lock of `{name}` names `{cap_name}`, which is not a capability in its module"
                    ),
                );
                continue;
            };
            let declared: Vec<&'a str> = env
                .capabilities
                .get(capability)
                .map(|cap| {
                    cap.ast
                        .ops
                        .iter()
                        .map(|op| env.text(cap.module, op.name.name))
                        .collect()
                })
                .unwrap_or_default();
            let ops: Vec<&'a str> = parsed
                .cap_type
                .ops
                .iter()
                .map(|op| env.text(module, op.name))
                .collect();
            if let Some(op) = ops.iter().find(|op| !declared.contains(op)) {
                report.error(
                    code,
                    clause.span,
                    format!(
                        "the lock of `{name}` lists `{op}`, which capability `{}` does not declare",
                        env.cap_display(capability)
                    ),
                );
                continue;
            }
            // A lock that labels an operation (ADR 0094 with ADR 0092 rule 8) is refused
            // rather than read as a lock that checks nothing: locked labels land with the
            // second part of #566.
            if parsed.cap_type.gives.iter().any(Option::is_some) {
                report.error(
                    code,
                    clause.span,
                    format!(
                        "the lock of `{name}` writes `gives` on an operation, which a lock cannot carry in this version"
                    ),
                );
                continue;
            }
            entries.push(Entry {
                clause: parsed,
                binding,
                capability,
                ops,
                span: clause.span,
                source: clause,
            });
        }
        env.locks.insert(id, entries);
    }
}

/// The public function that a lock's key names: `module.path.name`, declared in that module of
/// the package.
fn function(env: &Env<'_>, modules: &BTreeMap<String, usize>, qualified: &str) -> Option<usize> {
    let (path, name) = qualified.rsplit_once('.')?;
    let module = *modules.get(path)?;
    let Some(&Entity::Function(id)) = env.modules.get(module)?.lower.get(name) else {
        return None;
    };
    let def = env.functions.get(id)?;
    (def.module == module && def.public && def.kind == FnKind::Function).then_some(id)
}

/// Where the header of the function `def` grants `op` of the binding that `entry` locks: the
/// operation as it is written, or the whole clause when it lists no operations; the condition
/// the header writes on it; and whether the header lists it by name. `None` when the header
/// leaves the operation out.
pub(crate) fn header_site<'a>(
    env: &Env<'a>,
    def: &super::env::FnDef<'a>,
    entry: &Entry<'a>,
    op: &str,
) -> Option<(Span, Option<&'a una_syntax::ast::Expr>, bool)> {
    let FnAst::Function(decl) = def.ast else {
        return None;
    };
    let need = def.uses.iter().find(|need| {
        need.usage.name == entry.binding && need.usage.capability == Some(entry.capability)
    })?;
    if !need.usage.ops.iter().any(|granted| granted == op) {
        return None;
    }
    let clause = decl.clauses.iter().find_map(|clause| match &clause.kind {
        ClauseKind::Uses(uses) if env.text(def.module, uses.cap.name) == entry.binding => {
            Some(uses)
        }
        _ => None,
    })?;
    let written = clause
        .cap_type
        .ops
        .iter()
        .zip(&clause.cap_type.conditions)
        .find(|(ident, _)| env.text(def.module, ident.name) == op);
    Some(match written {
        Some((ident, condition)) => (ident.span, condition.as_ref(), true),
        None => (clause.span, None, false),
    })
}

/// U3023 for each binding and operation of a locked header that its lock does not grant: a
/// binding that no entry names with its capability, and an operation that its entry does not
/// list. Bindings and operations that the header leaves out are stronger, and pass.
pub(crate) fn drift(env: &Env<'_>, report: &mut Report<'_>) {
    for (&id, entries) in &env.locks {
        let Some(def) = env.functions.get(id) else {
            continue;
        };
        let FnAst::Function(decl) = def.ast else {
            continue;
        };
        if !env.reports(def.module) {
            continue;
        }
        let name = def.name;
        let clauses = decl.clauses.iter().filter_map(|clause| match &clause.kind {
            ClauseKind::Uses(uses) => Some(uses),
            _ => None,
        });
        for clause in clauses {
            let binding = env.text(def.module, clause.cap.name);
            let Some(need) = def.uses.iter().find(|need| need.usage.name == binding) else {
                continue;
            };
            // A capability that does not resolve is reported where it is written.
            let Some(capability) = need.usage.capability else {
                continue;
            };
            let entry = entries
                .iter()
                .find(|entry| entry.binding == binding && entry.capability == capability);
            let Some(entry) = entry else {
                let mut diagnostic = report.make(
                    Code::U3023,
                    clause.span,
                    format!(
                        "the header of `{name}` has `@{binding}: {}`, which its lock in `una.toml` does not name",
                        env.cap_display(capability)
                    ),
                );
                diagnostic.repairs.push(Report::advice(format!(
                    "remove `@{binding}` from the header of `{name}`, or bring the header back to its lock"
                )));
                report.push(diagnostic);
                continue;
            };
            for op in &need.usage.ops {
                if entry.ops.contains(&op.as_str()) {
                    continue;
                }
                let span = clause
                    .cap_type
                    .ops
                    .iter()
                    .find(|ident| env.text(def.module, ident.name) == op)
                    .map_or(clause.span, |ident| ident.span);
                let mut diagnostic = report.make(
                    Code::U3023,
                    span,
                    format!(
                        "the header of `{name}` lets `@{binding}.{op}` run, which its lock in `una.toml` does not list"
                    ),
                );
                diagnostic.related.push(Related {
                    location: report.location(entry.span),
                    message: "the lock".to_owned(),
                });
                diagnostic.repairs.push(Report::advice(format!(
                    "list only the operations of the lock in the header's `@{binding}`"
                )));
                report.push(diagnostic);
            }
        }
    }
}

/// ADR 0092, amendment of 2026-10-15: the text that restores the lock's condition `locked`,
/// whose conjuncts are `wanted`, on a header operation: ` where C_lock` after an operation with
/// no condition, or `(C_src) and (C_lock)` in place of the header's condition `C_src`, given
/// with its conjuncts. `None` under (b), a condition holding a line feed, the only line break
/// of the lexer, and under (d), every conjunct of `locked` already one of the header's, token
/// for token.
pub(crate) fn restoration(
    locked: &str,
    wanted: &[&str],
    header: Option<(&str, &[&str])>,
) -> Option<String> {
    if locked.contains('\n') {
        return None;
    }
    let Some((written, have)) = header else {
        return Some(format!(" where {locked}"));
    };
    let have: Vec<Vec<String>> = have
        .iter()
        .map(|text| super::conditions::tokens(text))
        .collect();
    if wanted
        .iter()
        .all(|text| have.contains(&super::conditions::tokens(text)))
    {
        return None;
    }
    Some(format!("({written}) and ({locked})"))
}

#[cfg(test)]
mod tests {
    use super::restoration;

    #[test]
    fn a_restoration_conjoins_and_keeps_what_is_written() {
        assert_eq!(
            restoration("not dry_run", &["not dry_run"], None).as_deref(),
            Some(" where not dry_run")
        );
        assert_eq!(
            restoration(
                "not dry_run",
                &["not dry_run"],
                Some(("cents > 0 or sku > 0", &["cents > 0 or sku > 0"]))
            )
            .as_deref(),
            Some("(cents > 0 or sku > 0) and (not dry_run)")
        );
    }

    #[test]
    fn b_a_condition_with_a_line_feed_gets_no_restoration() {
        assert_eq!(
            restoration("(not\n dry_run)", &["(not\n dry_run)"], None),
            None
        );
        // U+2028 is not a line break to the lexer.
        assert_eq!(
            restoration("r == \"a\u{2028}b\"", &["r == \"a\u{2028}b\""], None).as_deref(),
            Some(" where r == \"a\u{2028}b\"")
        );
    }

    #[test]
    fn d_a_condition_already_written_gets_no_restoration() {
        // Every conjunct of the lock's condition is one of the header's, with other layout.
        assert_eq!(
            restoration(
                "not dry_run and cents>0",
                &["not dry_run", "cents>0"],
                Some((
                    "(cents > 0) and (not  dry_run) and sku > 0",
                    &["cents > 0", "not  dry_run", "sku > 0"]
                ))
            ),
            None
        );
        // One conjunct missing: the whole condition is conjoined.
        assert_eq!(
            restoration(
                "not dry_run and cents > 0",
                &["not dry_run", "cents > 0"],
                Some(("not dry_run", &["not dry_run"]))
            )
            .as_deref(),
            Some("(not dry_run) and (not dry_run and cents > 0)")
        );
    }
}
