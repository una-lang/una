//! Types (M4): kinds and written types (U2002, U2003), bidirectional checking with unification
//! (U2001), calls and constructors (U2004 to U2006, U2009), `when` (U2007, U2008), statements
//! (U2011), the positions of `result`, `it` and `error` (U2012, U2013), the refinement language
//! (U2014), predicates (U2015) and lambdas (U2010), following `spec/03`, `spec/04`, `spec/06`,
//! `spec/10` and ADR 0024.
//!
//! Capabilities, effects and failures (M5): capability contexts, sufficiency, inference of the
//! headers of private functions, handlers, failures, one effect per line and the second-class
//! rules (U3001 to U3015, U7003), following `spec/05`, `spec/06`, `spec/10 §5, §6, §9` and
//! ADR 0028.
//!
//! The checker also records the proof obligations that refinement subsumption and the other
//! rules of `spec/12 §3` create. They stay pending until the verifier (M8) discharges them.

mod authority;
mod boundary;
mod call;
mod check;
mod conditions;
mod crossing;
mod dead;
mod edit_labels;
mod effects;
mod elab;
mod env;
mod expr;
mod facts;
mod fixed;
mod flow;
mod generic;
mod hover;
mod index;
mod label;
mod locals;
mod lock;
mod map;
mod memory;
mod records;
mod shared;
#[cfg(test)]
mod sharing_tests;
mod stmt;
mod summary;
mod ty;
mod unify;
mod units;
mod via;

pub use hover::{Hover, HoverStats, HoverType};
pub use index::Index;
use una_diag::{Code, Diagnostic, Edit, Escape, Location, ObligationKind, Related, Repair};
use una_project::Edition;
use una_syntax::ast::File;
use una_syntax::{FileId, Interner, KeywordSet, SourceDb, Span, lex_in, parse};

use crate::names::PackageInput;
use env::{Env, Source};

/// The prelude (`spec/14 §1, §3`) as Una source.
const PRELUDE: &str = include_str!("../std/prelude.una");
/// The declarations that edition 0.2 adds to the prelude (`spec/17 §8`), appended to it.
const PRELUDE_0_2: &str = include_str!("../std/prelude_0_2.una");

/// The rewrites of the prelude in edition 0.2, each a text of `PRELUDE` and its replacement:
/// `map`, `filter`, `first_where` and `fold` take effectful and failing functions (ADR 0062,
/// `spec/14 §1`), and `min`, `max`, `contains` and `sum` are generic over constrained types
/// (ADR 0066, `spec/20 §6`). Each text occurs exactly once in `PRELUDE` (a test checks it), so
/// an edit of the prelude can never turn a rewrite off silently.
const OF: [(&str, &str); 8] = [
    (
        "pub min(a: Int, b: Int) -> Int",
        "pub min[T: Ord](a: T, b: T) -> T",
    ),
    (
        "pub max(a: Int, b: Int) -> Int",
        "pub max[T: Ord](a: T, b: T) -> T",
    ),
    (
        "pub contains[T](xs: List[T], item: T) -> Bool",
        "pub contains[T: Eq](xs: List[T], item: T) -> Bool",
    ),
    (
        "pub sum(xs: List[Int]) -> Int = 0",
        "pub sum[T: Integer](xs: List[T]) -> Int = 0",
    ),
    (
        "  ensures result.len == xs.len\n=\n  []",
        "  ensures result.len == xs.len\n  uses of each\n  fails of each\n=\n  []",
    ),
    (
        "  ensures result.len <= xs.len\n=\n  xs",
        "  ensures result.len <= xs.len\n  uses of keep\n  fails of keep\n=\n  xs",
    ),
    (
        "keep: fn(T) -> Bool) -> Option[T] = None",
        "keep: fn(T) -> Bool) -> Option[T]\n  uses of keep\n  fails of keep\n=\n  None",
    ),
    (
        "step: fn(U, T) -> U) -> U = initial",
        "step: fn(U, T) -> U) -> U\n  uses of step\n  fails of step\n=\n  initial",
    ),
];

/// The prelude of edition 0.2: `PRELUDE` with the rewrites of `OF`.
fn of_prelude() -> String {
    OF.iter().fold(PRELUDE.to_owned(), |text, (from, to)| {
        text.replace(from, to)
    })
}

/// `std.testing` (`spec/14 §4`) as Una source.
const TESTING: &str = include_str!("../std/testing.una");

/// The standard modules as the checker reads them in one edition: the prelude's source, with
/// the declarations edition 0.2 rewrites and adds, `std.testing`'s source, and the keyword
/// set the prelude is lexed with (`spec/14`). Read-only: the checker loads exactly this text
/// (#648, the standard library reference).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandardSource {
    /// The prelude's source in this edition.
    pub prelude: String,
    /// `std.testing`'s source, the same in every edition.
    pub testing: &'static str,
    /// The keywords the prelude is lexed with; `std.testing` is lexed with those of 0.1.
    pub keywords: KeywordSet,
}

/// The built-in methods and properties that the checker resolves by the type of their
/// receiver, as `(receiver, name)`: `.len` on `Text`, `List` and `Map` (`spec/14 §2`), the
/// methods of maps (`spec/21 §2`), the conversions (`spec/18 §4`), and the wrapping,
/// saturating and bit operations of the fixed-width integers (`spec/18 §5`). The receivers are
/// `"len"`, `"Map"`, `"conversion"` and `"fixed"`. Read-only: it is built from the tables the
/// checker resolves them with, for the standard library reference (#690).
pub fn builtin_methods() -> Vec<(&'static str, String)> {
    let mut methods = vec![("len", "len".to_owned())];
    methods.extend(
        map::MAP_METHODS
            .iter()
            .map(|(name, ..)| ("Map", (*name).to_owned())),
    );
    methods.extend(una_core::IntType::ALL.iter().map(|t| {
        (
            "conversion",
            format!("to_{}", t.as_str().to_ascii_lowercase()),
        )
    }));
    methods.push(("conversion", "to_int".to_owned()));
    methods.extend(
        una_core::FixedOp::ALL
            .iter()
            .map(|op| ("fixed", op.as_str().to_owned())),
    );
    methods
}

/// The standard modules' source in `edition`, as [`Standard::load`] parses it.
pub fn standard_source(edition: Edition) -> StandardSource {
    let (prelude, keywords) = if edition >= Edition::V0_2 {
        (format!("{}\n{PRELUDE_0_2}", of_prelude()), KeywordSet::V0_2)
    } else {
        (PRELUDE.to_owned(), KeywordSet::V0_1)
    };
    StandardSource {
        prelude,
        testing: TESTING,
        keywords,
    }
}

/// A proof obligation that the type checker found (`spec/12 §3`). It stays pending until the
/// verifier generates it over Core IR and discharges it (M8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingObligation {
    /// The kind of obligation.
    pub kind: ObligationKind,
    /// The primary span of `spec/12 §3`: the value, the call, the returned expression, the
    /// division or the `when` keyword.
    pub location: Location,
    /// The declared type the obligation comes from, when it is written in the package.
    pub origin: Option<Location>,
}

/// A capability that a header uses: `uses @cap: capability[ops]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderUse {
    /// The reference, without `@`.
    pub cap: String,
    /// The capability's name.
    pub capability: String,
    /// The operations, in the capability's declaration order.
    pub ops: Vec<String>,
}

/// The authority and failure contract of a function of the package after inference
/// (`spec/04 §5`), which `una contract` prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// The module path, for example `events.checkin`.
    pub module: String,
    /// The function's name.
    pub name: String,
    /// Its `uses`, declared or inferred, in order.
    pub uses: Vec<HeaderUse>,
    /// Whether its `uses` are inferred: a private function without `uses` clauses.
    pub uses_inferred: bool,
    /// The name of its error kind, declared or inferred, when it can fail.
    pub fails: Option<String>,
    /// Whether its `fails` is inferred: a private function without a `fails` clause.
    pub fails_inferred: bool,
    /// The parameters of its `uses of` clauses, written or inferred (ADR 0062).
    pub uses_of: Vec<String>,
    /// The parameter of its `fails of` clause, written or inferred.
    pub fails_of: Option<String>,
    /// Its constrained type parameters, in order: the parameter, the constraint, and whether
    /// it is inferred (`spec/20 §8`).
    pub constraints: Vec<(String, String, bool)>,
    /// Its result summary: each position of its declared result whose summary is not empty,
    /// written as `spec/13 §5` writes it, with the type parameters there (ADR 0067).
    pub result_labels: Vec<(String, Vec<String>)>,
    /// Whether it has type parameters.
    pub generic: bool,
    /// Each parameter that an `of` clause names whose calls depend on decisions on type
    /// parameters, with those type parameters: callers run their argument under them
    /// (ADR 0067 amendment).
    pub of_raises: Vec<(String, Vec<String>)>,
}

/// The outcome of [`check_types`].
#[derive(Clone, Debug)]
pub struct TypeCheck {
    /// The type diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// The pending proof obligations, by position, then kind.
    pub obligations: Vec<PendingObligation>,
    /// Where each `let` whose right-hand side is in the refinement language binds its name.
    /// Such a binding has the selfified type `{v: B | v == e}` (`spec/03 §7`).
    pub selfified: Vec<Location>,
    /// The lambdas given to `of` parameters (ADR 0062), by span: whether the body performs
    /// effects and whether it can fail. Their function types in Core carry neither, since they
    /// are their caller's; `una core` exports them (#443).
    pub of_lambdas: Vec<(Span, bool, bool)>,
    /// The package's file-state label (ADR 0085 decision 1): what decides its `Files.write`
    /// calls, which labels the failure of every `Files` operation; `public` when it writes no
    /// file. `una core` exports it (#467), and the formal track's re-check checks every write
    /// against it.
    pub file_state: una_core::Label,
    /// The headers of the functions of the modules checked, by module, then declaration.
    pub headers: Vec<Header>,
    /// The Core IR of the declarations without diagnostics (`spec/09`).
    pub program: una_core::Program,
    /// What the checker resolved, by span, for the language server (ADR 0063).
    pub index: Index,
}

impl Default for TypeCheck {
    /// An empty outcome, whose file-state label is that of a package that writes no file.
    fn default() -> Self {
        TypeCheck {
            diagnostics: Vec::new(),
            obligations: Vec::new(),
            selfified: Vec::new(),
            of_lambdas: Vec::new(),
            file_state: una_core::Label::public(),
            headers: Vec::new(),
            program: una_core::Program::default(),
            index: Index::default(),
        }
    }
}

/// A declaration that the checker rejected, with its Core IR (ADR 0091): its type pass reported
/// a diagnostic and its elaboration is complete. `una core --with-rejected` exports the ones
/// whose every diagnostic is in the `security` domain, and nothing else reads them.
#[derive(Clone, Debug, PartialEq)]
pub struct Rejected {
    /// The path of its module.
    pub module: String,
    /// The span of the declaration, which holds its diagnostics.
    pub span: Span,
    /// Its Core IR.
    pub decl: una_core::Decl,
}

/// Type-checks the modules of `package` that have no syntax errors.
pub fn check_types(package: &PackageInput<'_>) -> TypeCheck {
    check_types_and(package, false).0
}

/// [`check_types`], and the Core IR of each rejected declaration whose elaboration is complete
/// (ADR 0091), for `una core --with-rejected`. The [`TypeCheck`] is the one [`check_types`]
/// returns: the rejected declarations are elaborated apart, after it, from what the checks
/// recorded, and change nothing in it.
pub fn check_types_with_rejected(package: &PackageInput<'_>) -> (TypeCheck, Vec<Rejected>) {
    check_types_and(package, true)
}

fn check_types_and(package: &PackageInput<'_>, with_rejected: bool) -> (TypeCheck, Vec<Rejected>) {
    let standard = Standard::load(package.settings.edition);
    let mut report = Report::new(package.sources);
    let sources = package
        .modules
        .iter()
        .map(|module| Source {
            tree: module.tree,
            interner: package.interner,
            reports: !module.broken,
        })
        .collect();
    let categories = package
        .settings
        .categories
        .iter()
        .map(|category| category.value.clone())
        .collect();
    let mut env = Env::build(
        sources,
        standard.source(&standard.prelude),
        standard.source(&standard.testing),
        categories,
        package.settings.edition,
        &mut report,
    );
    env.trusted_listed = package
        .settings
        .trusted
        .iter()
        .map(|entry| entry.value.clone())
        .collect();
    // Edition 0.3: the boundaries of `una.toml`, checked against the package (ADR 0093).
    env.boundaries = boundary::resolve(
        &env,
        package.settings,
        package.manifest,
        package.modules.len(),
        &mut report,
    );
    if !env.boundaries.list.is_empty() {
        env.boundaries.any_policy = env
            .functions
            .iter()
            .any(|f| matches!(f.ast, env::FnAst::Policy(_)));
        env.boundaries.function_kinds = crossing::function_kinds(&env);
    }
    let vias = via::plain_vias(&env, &report);
    env.plain_vias = vias.plain;
    env.blocked_vias = vias.blocked;
    let facts = facts::KindFacts::new(&env);
    facts::kind_recursion(&env, &mut report);
    authority::stored_effects(&env, &mut report);
    // Pass 1 collects what inference needs, and its diagnostics are dropped;
    // pass 2 checks with the inferred headers.
    // Pass 1 also records the result summaries of generic functions (ADR 0067); in edition
    // 0.2 they then reach their least fixpoint, below.
    let mut collected = authority::Collected::default();
    let mut edges = Vec::new();
    {
        let mut scratch = Report::new(package.sources);
        let mut findings = Findings::default();
        for module in 0..package.modules.len() {
            if env.reports(module) {
                check::module(
                    &mut env,
                    &facts,
                    &mut scratch,
                    &mut findings,
                    &mut collected,
                    authority::Pass::Collect,
                    module,
                );
            }
        }
        edges.extend(
            findings
                .edges
                .iter()
                .map(|(caller, callee, _)| (*caller, *callee)),
        );
        env.opaque_plain = opaque_plain(&env, &findings.edges);
    }
    if package.settings.edition >= Edition::V0_2 {
        summary_fixpoint(&mut env, &facts, package.sources, &edges);
        // ADR 0075 rule 5: once the summaries are final, pass 1 runs once more over the
        // declarations, so the label variables of editable bindings count every callee's final
        // summary. Its diagnostics and what it collects are dropped. Should that change a
        // solution, the summaries that read it are brought to their fixpoint again, so none
        // keeps a stale label; solutions only grow, so this ends.
        // Each round that goes on raises at least one solution, which rises only through the
        // finitely many labels the package can form; a round past the number of variables and
        // two more would mean they do not rise, so it is an internal error, not a silent stop
        // that could keep a stale label.
        let mut rounds = 0;
        while env.edit_labels_seen {
            rounds += 1;
            // Each round that goes on raises some variable by at least one atom, or records a
            // variable's shape once; atoms: the four levels above `public`, the categories, and
            // the symbolic labels the solutions name (ADR 0075 rule 5).
            let atoms = 4
                + env.categories.len()
                + env
                    .edit_labels
                    .values()
                    .flat_map(|label| label.symbols.iter())
                    .collect::<std::collections::BTreeSet<_>>()
                    .len();
            if rounds > env.edit_labels.len() * (atoms + 1) + 2 {
                let span = env
                    .edit_labels_changed_name
                    .as_ref()
                    .and_then(|name| env.edit_label_spans.get(name).copied())
                    .or_else(|| env.edit_label_spans.values().next().copied());
                if let Some(span) = span {
                    report.error(
                        Code::U8001,
                        span,
                        "internal error: the label variables of this editable binding did not reach a fixpoint (ADR 0075)".to_owned(),
                    );
                }
                break;
            }
            env.edit_labels_changed = false;
            let mut scratch = Report::new(package.sources);
            let mut findings = Findings::default();
            let mut collected = authority::Collected::default();
            for module in 0..package.modules.len() {
                if env.reports(module) {
                    check::module(
                        &mut env,
                        &facts,
                        &mut scratch,
                        &mut findings,
                        &mut collected,
                        authority::Pass::Collect,
                        module,
                    );
                }
            }
            if !env.edit_labels_changed {
                break;
            }
            summary_fixpoint(&mut env, &facts, package.sources, &edges);
        }
    }
    authority::infer(&mut env, &collected, &mut report);
    // Edition 0.3 (ADR 0100 rule 3): the functions whose inferred failure rests on code in
    // error, once the capabilities of their operations are inferred.
    let in_error = dead::in_error(&env, &collected);
    // Edition 0.3 (ADR 0092 rule 8, as amended): the locks of `[contracts]`, and the bindings
    // and operations of the locked headers that they do not grant.
    lock::resolve(&mut env, package.locks, &mut report);
    lock::drift(&env, &mut report);
    // S-14 (ADR 0085): a package that writes files labels the failure of every `Files`
    // operation with what decides its writes. Pass 2 is probed first, its diagnostics dropped,
    // so that a read checked before a write has the label too. The probe's label is the least
    // fixpoint: what the label itself raises in pass 2 is below it.
    if env.files_written {
        let mut scratch = Report::new(package.sources);
        let mut findings = Findings::default();
        let mut unused = authority::Collected::default();
        for module in 0..package.modules.len() {
            if env.reports(module) {
                check::module(
                    &mut env,
                    &facts,
                    &mut scratch,
                    &mut findings,
                    &mut unused,
                    authority::Pass::Check,
                    module,
                );
            }
        }
        env.file_state = file_state(&env).0;
        env.file_writes.clear();
        env.file_calls.clear();
        env.file_escaped.clear();
    }
    let mut findings = Findings::default();
    let mut unused = authority::Collected::default();
    for module in 0..package.modules.len() {
        if env.reports(module) {
            check::module(
                &mut env,
                &facts,
                &mut report,
                &mut findings,
                &mut unused,
                authority::Pass::Check,
                module,
            );
        }
    }
    // The label pass 2 found is never above the probe's: an internal error otherwise, never a
    // read whose failure the label would have missed.
    let (found, raised) = file_state(&env);
    if !found.flows_to(&env.file_state)
        && let Some(span) = raised
    {
        report.error(
            Code::U8001,
            span,
            format!(
                "internal error: this write raises the file-state label to `{found}`, above the `{}` that the checks used (ADR 0085)",
                env.file_state
            ),
        );
    }
    // The standard modules are checked silently, for the records that their
    // contracts are elaborated from.
    {
        let mut scratch = Report::new(&standard.sources);
        let mut standard_findings = Findings::default();
        for module in env.prelude..env.modules.len() {
            check::module(
                &mut env,
                &facts,
                &mut scratch,
                &mut standard_findings,
                &mut unused,
                authority::Pass::Check,
                module,
            );
        }
        findings.standard_records = standard_findings.records;
    }
    authority::testing_installs(&findings, &mut report);
    // Edition 0.3: no release is reachable from a call into a boundary (ADR 0093 rule 8).
    crossing::releases(&env, &findings, &mut report);
    facts::predicate_recursion(&env, &findings, &mut report);
    if package.settings.edition >= Edition::V0_2 {
        facts::generic_constraints(&mut env, &facts, &findings, &mut report);
    } else {
        facts::generic_equality(&env, &facts, &findings, &mut report);
    }
    authority::grants(&env, package.settings, package.run, &mut report);
    authority::budget(&env, &findings, package.settings, &mut report);
    // Edition 0.3 (ADR 0100): last, once every other error of the package is known.
    dead::judge(
        &mut findings,
        &in_error,
        package.earlier_errors,
        &mut report,
    );
    // Pending obligations by position, then kind.
    findings
        .obligations
        .sort_by_key(|(kind, span, _)| (span.file, span.start, span.end, *kind));
    // A span may be charged an `effect_condition` by more than one route, such as a call's
    // callee and a value it is passed (#615): one pending entry each.
    findings
        .obligations
        .dedup_by(|a, b| a.0 == ObligationKind::EffectCondition && a == b);
    let headers = headers(&env);
    let (program, lost) = elab::program(&env, &findings, package.sources, &standard.sources);
    // ADR 0103: the pending `refinement` obligations of the flows into refined positions,
    // decided by the rule tier 1 uses (#615).
    let kinds = una_core::flow::Kinds::new(&program);
    let flows = elab::refined_flows(&env, &findings, package.sources, &standard.sources);
    let before = findings.obligations.len();
    for (span, target, source) in flows {
        if kinds.pending(&source, &target) {
            findings
                .obligations
                .push((ObligationKind::Refinement, span, None));
        }
    }
    if findings.obligations.len() != before {
        findings
            .obligations
            .sort_by_key(|(kind, span, _)| (span.file, span.start, span.end, *kind));
    }
    // ADR 0038 D1: a component of a checker type that Core IR lost is an internal error, never
    // a silent pass (`spec/10 §2`). D-144: switched to B, ADR 0045: the
    // check runs in every build, not only in debug builds, and reports at the declaration.
    for (span, what) in lost {
        report.error(
            Code::U8001,
            span,
            format!(
                "internal error: the Core IR of this declaration lost {what} of its checked types"
            ),
        );
    }
    let index = index::build(&env, &findings);
    // ADR 0091 decision 5: elaborated only on request, after the program, and read by nothing
    // but `una core --with-rejected`.
    let rejected = if with_rejected {
        elab::rejected(&env, &findings, package.sources, &standard.sources)
    } else {
        Vec::new()
    };
    let check = TypeCheck {
        index,
        headers,
        program,
        obligations: findings
            .obligations
            .iter()
            .map(|(kind, span, origin)| PendingObligation {
                kind: *kind,
                location: report.location(*span),
                origin: origin.map(|origin| report.location(origin)),
            })
            .collect(),
        selfified: findings
            .selfified
            .iter()
            .map(|span| report.location(*span))
            .collect(),
        of_lambdas: findings.of_lambdas.clone(),
        file_state: env.file_state.clone(),
        diagnostics: report.diagnostics,
    };
    (check, rejected)
}

/// The headers of the functions of the modules that report, after inference.
fn headers(env: &Env<'_>) -> Vec<Header> {
    env.functions
        .iter()
        .enumerate()
        .filter(|(_, f)| f.kind == env::FnKind::Function && env.reports(f.module))
        .map(|(id, f)| Header {
            generic: !f.type_params.is_empty(),
            of_raises: env
                .of_raises
                .range((id, 0)..=(id, usize::MAX))
                .filter(|(_, params)| !params.is_empty())
                .filter_map(|((_, index), params)| {
                    let param = f.params.get(*index)?;
                    Some((
                        param.name.to_owned(),
                        params
                            .iter()
                            .filter_map(|p| f.type_params.get(*p))
                            .map(|p| (*p).to_owned())
                            .collect(),
                    ))
                })
                .collect(),
            result_labels: env
                .summaries
                .get(&id)
                .map(|summary| {
                    summary
                        .iter()
                        .filter(|(_, params)| !params.is_empty())
                        .map(|(path, params)| {
                            (
                                env::position_text(path),
                                params
                                    .iter()
                                    .filter_map(|index| f.type_params.get(*index))
                                    .map(|param| (*param).to_owned())
                                    .collect(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            module: env
                .modules
                .get(f.module)
                .map(|m| m.path.clone())
                .unwrap_or_default(),
            name: f.name.to_owned(),
            constraints: f
                .type_params
                .iter()
                .zip(&f.constraints)
                .zip(&f.constraints_written)
                .filter(|((_, constraint), _)| **constraint != env::Constraint::None)
                .map(|((param, constraint), written)| {
                    ((*param).to_owned(), constraint.name().to_owned(), !written)
                })
                .collect(),
            uses: f
                .uses
                .iter()
                .map(|need| HeaderUse {
                    cap: need.usage.name.clone(),
                    capability: need
                        .usage
                        .capability
                        .and_then(|c| env.capabilities.get(c))
                        .map_or_else(String::new, |c| c.name.to_owned()),
                    ops: need.usage.ops.clone(),
                })
                .collect(),
            uses_inferred: f.infer_uses,
            fails: match &f.fails {
                Some(ty::Ty::Kind(id, _)) => Some(env.kind_name(*id).to_owned()),
                _ => None,
            },
            fails_inferred: f.infer_fails,
            uses_of: f
                .uses_of
                .iter()
                .filter_map(|index| f.params.get(*index))
                .map(|param| param.name.to_owned())
                .collect(),
            fails_of: f
                .fails_of
                .and_then(|index| f.params.get(index))
                .map(|param| param.name.to_owned()),
        })
        .collect()
}

/// What checking the bodies finds besides diagnostics.
#[derive(Default)]
pub(crate) struct Findings {
    /// Pending obligations: kind, primary span and the span of the declared type.
    pub(crate) obligations: Vec<(ObligationKind, Span, Option<Span>)>,
    /// The edits of a callee's condition that the sites of `effect_condition` obligations ask
    /// for, each with its callee, before condition (d)'s function values are known (ADR 0092,
    /// amendment of 2026-10-14).
    pub(crate) condition_edits: Vec<(conditions::EditTarget, una_core::ConditionEdit)>,
    /// The functions named as values anywhere in the package (ADR 0092 rule 1).
    pub(crate) function_values: std::collections::BTreeSet<usize>,
    /// Names bound by selfified `let`s.
    pub(crate) selfified: Vec<Span>,
    /// Calls of, and references to, functions and predicates, from the body of a function or
    /// predicate: caller, callee, and where.
    pub(crate) edges: Vec<(usize, usize, Span)>,
    /// Edition 0.3 (ADR 0093 rule 8): what each function and handler may run, by named call,
    /// function value or installation, for the releases reachable from a boundary call.
    pub(crate) graph: Vec<(crossing::Node, crossing::Node)>,
    /// The releases, `declassify` and policy calls, of each function and handler.
    pub(crate) releases: Vec<(crossing::Node, Span)>,
    /// The handlers given a function value whose code is not known where they are installed.
    pub(crate) opaque: Vec<(crossing::Node, Span)>,
    /// What the arguments of each installation of a handler outside every boundary grant it,
    /// by the span that its binding records: a boundary that runs the handler runs them too.
    pub(crate) installed: std::collections::BTreeMap<Span, crossing::Installed>,
    /// The calls into boundary modules.
    pub(crate) crossings: Vec<crossing::Crossing>,
    /// Edition 0.3 (ADR 0093 rule 15): the summary of what each binding leads to. An edge
    /// `(from, to)` says that a call into a boundary that reaches `from` reaches `to`.
    pub(crate) summary_edges: Vec<(crossing::Source, crossing::Source)>,
    /// What each call into a boundary grants by name: the binding, the boundary and the call.
    pub(crate) summary_seeds: Vec<(crossing::Source, usize, Span)>,
    /// The handlers installed outside every boundary, checked once the summary is closed.
    pub(crate) install_sites: Vec<crossing::InstallSite>,
    /// Instantiations of generic functions: the caller when it is a function, the callee, its
    /// type arguments and where.
    pub(crate) instances: Vec<(Option<usize>, usize, Vec<ty::Ty>, Span)>,
    /// The lambdas given to `of` parameters, checked in their caller's context (ADR 0062): the
    /// span of each, whether its body performs effects, and whether it can fail.
    pub(crate) of_lambdas: Vec<(Span, bool, bool)>,
    /// For each generic function, the type parameters whose values its body compares with
    /// `==` or `!=`.
    pub(crate) compared: std::collections::BTreeMap<usize, std::collections::BTreeSet<usize>>,
    /// Edition 0.2: for each private function, the constraint its body needs on each type
    /// parameter without a written one (`spec/20 §3` rule 4).
    pub(crate) needs:
        std::collections::BTreeMap<usize, std::collections::BTreeMap<usize, env::Constraint>>,
    /// The uses of `checked`, by module, in order (`spec/06 §6`).
    pub(crate) checked: Vec<(usize, Span)>,
    /// What pass 2 resolves in the package's modules, for elaboration.
    pub(crate) records: records::Records,
    /// What the silent check of the standard modules resolves, for the elaboration of their
    /// contracts. The standard modules have a source database of their own, whose file ids
    /// coincide with the package's, so their records by span are kept apart (flow sweep).
    pub(crate) standard_records: records::Records,
    /// The declarations of pass 2 with a diagnostic, by the span of their name.
    pub(crate) failed: std::collections::BTreeSet<Span>,
    /// For each handler whose operation bodies use `std.testing`, the first use (ADR 0043).
    pub(crate) testing_handlers: std::collections::BTreeMap<usize, Span>,
    /// The handlers installed by `with` bindings, and the span of each binding (ADR 0043).
    pub(crate) with_installs: Vec<(usize, Span)>,
    /// Edition 0.3: the `if error` clauses and `attempt`s on code that cannot fail (ADR 0100),
    /// judged once the package is checked.
    pub(crate) dead: Vec<dead::Site>,
    /// The calls whose failure stops where they are: handled by `if error use`, by `if error
    /// fail` or by `attempt` (ADR 0100 rule 3).
    pub(crate) contained: std::collections::BTreeSet<Span>,
    /// The spans of the calls of declared functions, apart from their mentions as values.
    pub(crate) named_calls: std::collections::BTreeSet<Span>,
    /// The functions whose bodies have a site of U3007, U3008 or U3009, held ones included.
    pub(crate) family: std::collections::BTreeSet<usize>,
}

/// The parsed standard modules.
struct Standard {
    sources: SourceDb,
    interner: Interner,
    prelude: File,
    testing: File,
}

impl Standard {
    /// Parses the prelude and `std.testing`. They are part of the compiler and parse without
    /// errors (a test checks it); their spans belong to a database of their own and are never
    /// reported.
    fn load(edition: Edition) -> Standard {
        let mut sources = SourceDb::new();
        let mut interner = Interner::new();
        let StandardSource {
            prelude: text,
            testing,
            keywords,
        } = standard_source(edition);
        let prelude = sources.add("std/prelude.una", text.as_bytes());
        let testing = sources.add("std/testing.una", testing.as_bytes());
        let prelude = parse_standard(&sources, prelude, &mut interner, keywords);
        let testing = parse_standard(&sources, testing, &mut interner, KeywordSet::V0_1);
        Standard {
            sources,
            interner,
            prelude,
            testing,
        }
    }

    fn source<'a>(&'a self, tree: &'a File) -> Source<'a> {
        Source {
            tree,
            interner: &self.interner,
            reports: false,
        }
    }
}

fn parse_standard(
    sources: &SourceDb,
    id: FileId,
    interner: &mut Interner,
    keywords: KeywordSet,
) -> File {
    let lexed = lex_in(sources.file(id), id, interner, keywords);
    parse(&lexed, id, interner).file
}

/// The least fixpoint of the result summaries of the generic functions (ADR 0067 L2): over
/// the strongly connected components of their calls, callees first, each component's bodies
/// are checked again until none of their summaries grows. Summaries only grow, over finitely
/// many positions and type parameters, so each loop ends, and a function is checked again
/// only with its own component.
fn summary_fixpoint(
    env: &mut Env<'_>,
    facts: &facts::KindFacts,
    sources: &SourceDb,
    edges: &[(usize, usize)],
) {
    let generic: std::collections::BTreeSet<usize> = env
        .functions
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            f.module < env.prelude && f.kind == env::FnKind::Function && !f.type_params.is_empty()
        })
        .map(|(id, _)| id)
        .collect();
    let components = summary::components(&generic, edges);
    // The callers of each function within its own component, by component, from one pass over
    // the edges in their order (#375): O(E log n) in all, not a pass per component.
    let mut component_of: std::collections::BTreeMap<usize, usize> =
        std::collections::BTreeMap::new();
    for (index, component) in components.iter().enumerate() {
        for &function in component {
            component_of.insert(function, index);
        }
    }
    let mut callers_in: Vec<std::collections::BTreeMap<usize, Vec<usize>>> =
        vec![std::collections::BTreeMap::new(); components.len()];
    for &(caller, callee) in edges {
        if let (Some(&from), Some(&to)) = (component_of.get(&caller), component_of.get(&callee))
            && from == to
            && let Some(callers) = callers_in.get_mut(to)
        {
            callers.entry(callee).or_default().push(caller);
        }
    }
    // Where each function is declared, so that checking one does not scan the package (#375).
    let places = check::places(env);
    for (component, callers) in components.iter().zip(&callers_in) {
        // A worklist: a function is checked again only when the summary of a function it
        // calls has grown, so a cycle of n functions takes about n checks, not n rounds of n.
        let members: std::collections::BTreeSet<usize> = component.iter().copied().collect();
        let mut queue: std::collections::VecDeque<usize> = component.iter().copied().collect();
        let mut queued = members.clone();
        while let Some(function) = queue.pop_front() {
            queued.remove(&function);
            let before = summary::size(env, function);
            // A check that changes the solution of a label variable is followed by one more,
            // which reads the solution, so the summary counts the labels it gives (ADR 0075).
            for _ in 0..2 {
                env.edit_labels_changed = false;
                let mut scratch = Report::new(sources);
                let mut findings = Findings::default();
                let mut collected = authority::Collected::default();
                check::declarations_at(
                    env,
                    facts,
                    &mut scratch,
                    &mut findings,
                    &mut collected,
                    authority::Pass::Collect,
                    places.get(&function).map_or(&[][..], Vec::as_slice),
                );
                if !env.edit_labels_changed {
                    break;
                }
            }
            if summary::size(env, function) == before {
                continue;
            }
            for &caller in callers.get(&function).map_or(&[][..], Vec::as_slice) {
                if queued.insert(caller) {
                    queue.push_back(caller);
                }
            }
        }
    }
}

/// Collects diagnostics, with the sources their spans point into.
pub(crate) struct Report<'s> {
    sources: &'s SourceDb,
    diagnostics: Vec<Diagnostic>,
}

impl<'s> Report<'s> {
    fn new(sources: &'s SourceDb) -> Report<'s> {
        Report {
            sources,
            diagnostics: Vec::new(),
        }
    }

    /// The location of `span`, a span of a package module.
    pub(crate) fn location(&self, span: Span) -> Location {
        self.sources.location(span)
    }

    /// The source text of `span`.
    pub(crate) fn text(&self, span: Span) -> &'s str {
        self.sources
            .file(span.file)
            .text()
            .get(span.start..span.end)
            .unwrap_or("")
    }

    /// A diagnostic, to complete with repairs or related spans before [`Report::push`].
    pub(crate) fn make(&self, code: Code, span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic::error(code, message, self.location(span))
    }

    pub(crate) fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub(crate) fn error(&mut self, code: Code, span: Span, message: impl Into<String>) {
        let diagnostic = self.make(code, span, message);
        self.push(diagnostic);
    }

    /// Adds `repair` to the diagnostic with `code` at `span` among those reported since the
    /// first `mark` ones.
    pub(crate) fn repair_since(&mut self, mark: usize, code: Code, span: Span, repair: Repair) {
        let location = self.location(span);
        if let Some(diagnostic) = self
            .diagnostics
            .iter_mut()
            .skip(mark)
            .find(|d| d.code == code && d.primary == location)
        {
            diagnostic.repairs.push(repair);
        }
    }

    /// Whether a diagnostic with `code` is already reported at `span`.
    pub(crate) fn has(&self, code: Code, span: Span) -> bool {
        let location = self.location(span);
        self.diagnostics
            .iter()
            .any(|d| d.code == code && d.primary == location)
    }

    /// How many diagnostics have been reported so far.
    pub(crate) fn count(&self) -> usize {
        self.diagnostics.len()
    }

    /// A repair that replaces `span` with `replacement`.
    pub(crate) fn edit(
        &self,
        description: impl Into<String>,
        span: Span,
        replacement: impl Into<String>,
    ) -> Repair {
        Repair {
            widens: None,
            rank: 1,
            escape: None,
            description: description.into(),
            edit: Some(Edit {
                location: self.location(span),
                replacement: replacement.into(),
            }),
        }
    }

    /// A repair described in words, without an edit.
    pub(crate) fn advice(description: impl Into<String>) -> Repair {
        Repair {
            widens: None,
            rank: 1,
            escape: None,
            description: description.into(),
            edit: None,
        }
    }

    /// A repair described in words that takes the escape hatch `escape`: output ranks it after
    /// every repair that keeps the guarantee (ADR 0095).
    pub(crate) fn escape(escape: Escape, description: impl Into<String>) -> Repair {
        Repair {
            escape: Some(escape),
            ..Self::advice(description)
        }
    }

    pub(crate) fn related(&self, span: Span, message: impl Into<String>) -> Related {
        Related {
            location: self.location(span),
            message: message.into(),
        }
    }
}

/// The opaque predicates of the checked modules whose bodies call no other opaque predicate
/// (ADR 0095): those of other packages, whose bodies were not checked, are left out. Linear in
/// the functions and the call edges.
fn opaque_plain(
    env: &Env<'_>,
    edges: &[(usize, usize, Span)],
) -> std::collections::BTreeSet<usize> {
    let opaque = |id: usize| {
        env.functions
            .get(id)
            .is_some_and(|def| def.kind == (env::FnKind::Predicate { reflect: false }))
    };
    let composite: std::collections::BTreeSet<usize> = edges
        .iter()
        .filter(|(caller, callee, _)| opaque(*caller) && opaque(*callee))
        .map(|(caller, _, _)| *caller)
        .collect();
    env.functions
        .iter()
        .enumerate()
        .filter(|(id, def)| opaque(*id) && env.reports(def.module) && !composite.contains(id))
        .map(|(id, _)| id)
        .collect()
}

/// The candidate closest to `name` by edit distance, when it is close enough to be the
/// intended name, for repairs.
pub(crate) fn closest<'n>(
    name: &str,
    candidates: impl IntoIterator<Item = &'n str>,
) -> Option<&'n str> {
    let limit = (name.chars().count() / 3).clamp(1, 3);
    let mut best: Option<(usize, &str)> = None;
    for candidate in candidates {
        let distance = edit_distance(name, candidate);
        if distance <= limit && best.is_none_or(|(known, _)| distance < known) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// The candidates close enough to `name` to be the intended name, by edit distance within the
/// limit of [`closest`], each once, in a fixed order: by distance, then bytewise; at most
/// three, each with its distance. U1001 offers them (#419).
pub(crate) fn nearest<'n>(
    name: &str,
    candidates: impl IntoIterator<Item = &'n str>,
) -> Vec<(usize, &'n str)> {
    let limit = (name.chars().count() / 3).clamp(1, 3);
    let mut close: Vec<(usize, &'n str)> = candidates
        .into_iter()
        .filter(|candidate| *candidate != name)
        .map(|candidate| (edit_distance(name, candidate), candidate))
        .filter(|(distance, _)| *distance <= limit)
        .collect();
    close.sort_unstable();
    close.dedup();
    close.truncate(3);
    close
}

/// The edit distance between `a` and `b` over scalar values, counting an insertion, a
/// deletion, a substitution or a transposition of two neighbors as one edit.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    // rows[i][j] is the distance between the first i scalars of `a` and the first j of `b`.
    let mut rows: Vec<Vec<usize>> = (0..=a.len())
        .map(|i| {
            (0..=b.len())
                .map(|j| {
                    if i == 0 {
                        j
                    } else if j == 0 {
                        i
                    } else {
                        0
                    }
                })
                .collect()
        })
        .collect();
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let at = |i: usize, j: usize, rows: &Vec<Vec<usize>>| {
                rows.get(i)
                    .and_then(|row| row.get(j))
                    .copied()
                    .unwrap_or(usize::MAX)
            };
            let same = a.get(i - 1) == b.get(j - 1);
            let mut best = at(i - 1, j, &rows)
                .saturating_add(1)
                .min(at(i, j - 1, &rows).saturating_add(1))
                .min(at(i - 1, j - 1, &rows).saturating_add(usize::from(!same)));
            if i > 1 && j > 1 && a.get(i - 1) == b.get(j - 2) && a.get(i - 2) == b.get(j - 1) {
                best = best.min(at(i - 2, j - 2, &rows).saturating_add(1));
            }
            if let Some(cell) = rows.get_mut(i).and_then(|row| row.get_mut(j)) {
                *cell = best;
            }
        }
    }
    rows.last().and_then(|row| row.last()).copied().unwrap_or(0)
}

/// The file-state label of the package from what a check of pass 2 recorded (S-14, ADR 0085),
/// and the first write that raises it above the label the checks used.
///
/// Each write contributes its path, its program counter and the highest program counter its
/// code can be entered under. A function's is the join of the program counters of its direct
/// calls, each raised by the caller's own, up to a fixpoint over the call graph; a function
/// used as a value, or a policy's `via` function, may be called anywhere under its effect
/// bound (`spec/07 §7` rule 4), `public` when it can fail; a handler operation's is its effect
/// floor, a lambda's the cap, an example's `public`. Everything is capped at what `Files.write`
/// accepts. The fixpoint ends: each round raises some function's label, within a finite
/// lattice.
fn file_state(env: &env::Env<'_>) -> (label::Label, Option<Span>) {
    use env::Entry;
    let cap = env.file_cap.clone();
    let mut entries: std::collections::BTreeMap<usize, label::Label> =
        std::collections::BTreeMap::new();
    for &f in env.file_escaped.iter().chain(env.vias.keys()) {
        let label = match env.functions.get(f) {
            Some(def) if def.fails.is_some() => label::Label::public(),
            Some(def) => {
                let uses: Vec<ty::FnUse> = def.uses.iter().map(|n| n.usage.clone()).collect();
                flow::bound_of(env, &uses).map_or_else(|| cap.clone(), |b| b.meet(&cap))
            }
            None => cap.clone(),
        };
        let current = entries.entry(f).or_insert_with(label::Label::public);
        *current = current.join(&label).meet(&cap);
    }
    let entry_of = |entries: &std::collections::BTreeMap<usize, label::Label>, e: &Entry| match e {
        Entry::Function(f) => entries.get(f).cloned().unwrap_or_else(label::Label::public),
        Entry::Floor(floor) => floor.meet(&cap),
        Entry::Lambda => cap.clone(),
        Entry::Root => label::Label::public(),
    };
    loop {
        let mut changed = false;
        for (caller, callee, pc) in &env.file_calls {
            let raised = pc.join(&entry_of(&entries, caller)).meet(&cap);
            let current = entries
                .get(callee)
                .cloned()
                .unwrap_or_else(label::Label::public);
            if !raised.flows_to(&current) {
                entries.insert(*callee, current.join(&raised).meet(&cap));
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut state = label::Label::public();
    let mut raised = None;
    for (entry, site, span) in &env.file_writes {
        let contribution = site.join(&entry_of(&entries, entry)).meet(&cap);
        if raised.is_none() && !contribution.flows_to(&env.file_state) {
            raised = Some(*span);
        }
        state = state.join(&contribution);
    }
    (state, raised)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rewrite_of_the_prelude_applies_exactly_once() {
        // `str::replace` does nothing when its text is missing, so an edit of the prelude could
        // turn a rewrite of edition 0.2 off without a word (#648).
        let rewritten = of_prelude();
        for (from, to) in OF {
            assert_eq!(
                PRELUDE.matches(from).count(),
                1,
                "the prelude holds {from:?}"
            );
            assert_eq!(rewritten.matches(to).count(), 1, "edition 0.2 holds {to:?}");
            assert_eq!(
                rewritten.matches(from).count(),
                0,
                "edition 0.2 still holds {from:?}"
            );
        }
    }

    #[test]
    fn the_standard_source_is_the_text_the_checker_loads() {
        // #648: the reference listing reads `standard_source`; it must be exactly the text
        // `Standard::load` parses, for every edition.
        for edition in &Edition::ALL {
            let source = standard_source(*edition);
            let (prelude, keywords) = if *edition >= Edition::V0_2 {
                (format!("{}\n{PRELUDE_0_2}", of_prelude()), KeywordSet::V0_2)
            } else {
                (PRELUDE.to_owned(), KeywordSet::V0_1)
            };
            assert_eq!(source.prelude, prelude, "{edition:?}");
            assert_eq!(source.testing, TESTING, "{edition:?}");
            assert_eq!(source.keywords, keywords, "{edition:?}");
            let texts = Standard::load(*edition).sources.texts();
            assert_eq!(texts.get("std/prelude.una"), Some(&prelude), "{edition:?}");
            assert_eq!(
                texts.get("std/testing.una").map(String::as_str),
                Some(TESTING),
                "{edition:?}"
            );
        }
    }

    #[test]
    fn the_standard_modules_parse_in_their_editions() {
        // A syntax error in a standard module would drop its declarations without a word, as
        // a keyword used as a name does.
        let v0_2 = format!("{}\n{PRELUDE_0_2}", of_prelude());
        for (name, text, keywords) in [
            ("prelude", PRELUDE, KeywordSet::V0_1),
            ("prelude 0.2", v0_2.as_str(), KeywordSet::V0_2),
            ("testing", TESTING, KeywordSet::V0_1),
        ] {
            let mut sources = SourceDb::new();
            let mut interner = Interner::new();
            let id = sources.add(name, text.as_bytes());
            let lexed = lex_in(sources.file(id), id, &mut interner, keywords);
            let parsed = parse(&lexed, id, &interner);
            assert!(lexed.errors.is_empty(), "{name}: {:?}", lexed.errors);
            assert!(parsed.errors.is_empty(), "{name}: {:?}", parsed.errors);
        }
    }

    #[test]
    fn the_prelude_error_kinds_the_formal_model_reads_as_codes_have_no_payload() {
        // #508: the formal re-check reads `FileError`, `KeyError` and `HttpError` as a
        // failure's code, the index of the variant (`formal/UnaFormal/F1/Import.lean`,
        // `codeKinds`), and renders such a value as that index. Both readings distinguish two
        // values exactly when their variants differ, which holds only while no variant carries
        // a field. Each kind must stay one line of bare variants, without type parameters, in
        // each edition's prelude; if one gains a payload, `codeKinds` must drop it first.
        let v0_2 = format!("{}\n{PRELUDE_0_2}", of_prelude());
        for (edition, text, kinds) in [
            ("0.1", PRELUDE, &["FileError", "KeyError"][..]),
            (
                "0.2",
                v0_2.as_str(),
                &["FileError", "KeyError", "HttpError"][..],
            ),
        ] {
            for kind in kinds {
                // Every declaration of the kind, in any form: `kind K =`, `kind K[T]`, or a
                // record kind whose fields follow on their own lines.
                let decl = format!("kind {kind}");
                let lines: Vec<&str> = text
                    .lines()
                    .filter(|line| {
                        line.match_indices(decl.as_str()).any(|(i, _)| {
                            !line[i + decl.len()..]
                                .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
                        })
                    })
                    .collect();
                assert_eq!(
                    lines.len(),
                    1,
                    "{edition}: {kind} is declared once: {lines:?}"
                );
                let head = format!("pub kind {kind} =");
                let variants = lines[0]
                    .strip_prefix(head.as_str())
                    .unwrap_or_else(|| panic!("{edition}: `{}` is not `{head} ...`", lines[0]));
                let variants: Vec<&str> = variants
                    .split('|')
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .collect();
                assert!(!variants.is_empty(), "{edition}: {kind} has variants");
                for v in variants {
                    assert!(
                        v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                        "{edition}: {kind}'s variant `{v}` carries a payload"
                    );
                }
            }
        }
    }

    #[test]
    fn every_kind_of_the_prelude_is_declared_in_core_or_known_to_every_engine() {
        // #407: the verifier and the native backend stop on a `Named` kind they cannot
        // resolve, so a sum or record of the prelude that Core does not declare must be one
        // each engine knows itself. A new kind of the prelude fails here until it is one or
        // the other, never both.
        use una_syntax::ast::{DeclKind, KindBody};
        let v0_2 = format!("{}\n{PRELUDE_0_2}", of_prelude());
        let mut kinds = Vec::new();
        for text in [PRELUDE, v0_2.as_str()] {
            let mut sources = SourceDb::new();
            let mut interner = Interner::new();
            let id = sources.add("prelude", text.as_bytes());
            let lexed = lex_in(sources.file(id), id, &mut interner, KeywordSet::V0_2);
            let parsed = parse(&lexed, id, &interner);
            for decl in &parsed.file.decls {
                if let DeclKind::Kind(kind) = &decl.kind
                    && !matches!(kind.body, KindBody::Alias { .. })
                {
                    kinds.push(interner.resolve(kind.name.name).to_owned());
                }
            }
        }
        assert!(kinds.contains(&"Entry".to_owned()), "{kinds:?}");
        for kind in &kinds {
            let core = elab::PRELUDE_CORE_KINDS.contains(&kind.as_str());
            let engine = una_core::PRELUDE_ENGINE_KINDS.contains(&kind.as_str());
            assert!(
                core != engine,
                "`{kind}`: declared in Core {core}, known to the engines {engine}"
            );
        }
        for kind in una_core::PRELUDE_ENGINE_KINDS {
            assert!(
                kinds.iter().any(|k| k == kind) || ["List", "Map"].contains(&kind),
                "`{kind}` is not a kind of the prelude"
            );
        }
    }

    #[test]
    fn the_engines_table_of_prelude_kinds_is_the_prelude_s_declarations() {
        // #415: the interpreter names, and the native backend describes, the fields of the
        // prelude's kinds that Core does not declare by `una_core::prelude_variants`. It must
        // be what the prelude declares: each variant, its fields, and each field's type, a
        // type parameter of the kind by position or `Int`, with no label.
        use una_core::PreludeField;
        use una_syntax::ast::{DeclKind, Field, KindBody, TypeCore};
        type Variants = Vec<(String, Vec<(String, Option<PreludeField>)>)>;
        let v0_2 = format!("{}\n{PRELUDE_0_2}", of_prelude());
        let mut sources = SourceDb::new();
        let mut interner = Interner::new();
        let id = sources.add("prelude", v0_2.as_bytes());
        let lexed = lex_in(sources.file(id), id, &mut interner, KeywordSet::V0_2);
        let parsed = parse(&lexed, id, &interner);
        let mut seen = Vec::new();
        for decl in &parsed.file.decls {
            let DeclKind::Kind(kind) = &decl.kind else {
                continue;
            };
            let name = interner.resolve(kind.name.name);
            let Some(table) = una_core::prelude_variants(name) else {
                continue;
            };
            seen.push(name.to_owned());
            let params: Vec<&str> = kind
                .params
                .iter()
                .map(|p| interner.resolve(p.name))
                .collect();
            let field = |field: &Field| -> (String, Option<PreludeField>) {
                assert!(field.ty.label.is_none(), "`{name}` declares a label");
                let of = match &field.ty.core {
                    TypeCore::Named { name: ty, args } if args.is_empty() => {
                        let ty = interner.resolve(ty.name);
                        match params.iter().position(|p| *p == ty) {
                            Some(position) => Some(PreludeField::Arg(position)),
                            None => (ty == "Int").then_some(PreludeField::Int),
                        }
                    }
                    _ => None,
                };
                (interner.resolve(field.name.name).to_owned(), of)
            };
            let declared: Variants = match &kind.body {
                KindBody::Sum(variants) => variants
                    .iter()
                    .map(|v| {
                        let fields = v.fields.iter().map(field).collect();
                        (interner.resolve(v.name.name).to_owned(), fields)
                    })
                    .collect(),
                KindBody::Record(fields) => {
                    vec![(name.to_owned(), fields.iter().map(field).collect())]
                }
                KindBody::Alias { .. } => panic!("`{name}` is an alias"),
            };
            let tabled: Variants = table
                .iter()
                .map(|(variant, fields)| {
                    let fields = fields
                        .iter()
                        .map(|(field, of)| ((*field).to_owned(), Some(*of)))
                        .collect();
                    ((*variant).to_owned(), fields)
                })
                .collect();
            assert_eq!(tabled, declared, "`{name}`");
        }
        let mut expected: Vec<String> = una_core::PRELUDE_ENGINE_KINDS
            .iter()
            .filter(|kind| !["List", "Map"].contains(kind))
            .map(|kind| (*kind).to_owned())
            .collect();
        expected.sort();
        seen.sort();
        assert_eq!(
            seen, expected,
            "every kind of the table is declared in the prelude"
        );
        assert!(una_core::prelude_variants("List").is_none());
        assert!(una_core::prelude_variants("Map").is_none());
    }

    /// The diagnostics of the standard modules of `edition`, checked as a package's are.
    fn standard_diagnostics(edition: Edition) -> Vec<String> {
        let standard = Standard::load(edition);
        let mut report = Report::new(&standard.sources);
        let source = |tree| Source {
            tree,
            interner: &standard.interner,
            reports: true,
        };
        let mut env = Env::build(
            Vec::new(),
            source(&standard.prelude),
            source(&standard.testing),
            Vec::new(),
            edition,
            &mut report,
        );
        let facts = facts::KindFacts::new(&env);
        facts::kind_recursion(&env, &mut report);
        let mut collected = authority::Collected::default();
        {
            let mut scratch = Report::new(&standard.sources);
            let mut findings = Findings::default();
            for module in env.prelude..env.modules.len() {
                check::module(
                    &mut env,
                    &facts,
                    &mut scratch,
                    &mut findings,
                    &mut collected,
                    authority::Pass::Collect,
                    module,
                );
            }
        }
        authority::infer(&mut env, &collected, &mut report);
        let mut findings = Findings::default();
        let mut unused = authority::Collected::default();
        for module in env.prelude..env.modules.len() {
            check::module(
                &mut env,
                &facts,
                &mut report,
                &mut findings,
                &mut unused,
                authority::Pass::Check,
                module,
            );
        }
        report
            .diagnostics
            .iter()
            .map(|d| format!("{:?} {}", d.code, d.message))
            .collect()
    }

    #[test]
    fn the_standard_modules_check_without_diagnostics_in_every_edition() {
        // A type or label error in a standard module would drop or weaken what agents call,
        // without a word; checked as a package's, they report nothing.
        for edition in Edition::ALL {
            assert_eq!(
                standard_diagnostics(edition),
                Vec::<String>::new(),
                "{edition:?}"
            );
        }
    }

    #[test]
    fn nearest_names_come_by_distance_then_bytewise_three_at_most() {
        // #419: U1001's candidates, deterministic whatever the order of the scopes.
        let scope = ["car", "can", "cap", "cat", "caz", "cart", "zebra"];
        assert_eq!(
            nearest("caz", scope),
            vec![(1, "can"), (1, "cap"), (1, "car")]
        );
        let reversed: Vec<&str> = scope.iter().rev().copied().collect();
        assert_eq!(nearest("caz", reversed), nearest("caz", scope));
        // The limit grows with the name, a third of its length, from 1 to 3.
        assert_eq!(nearest("cr", ["car", "cat"]), vec![(1, "car")]);
        assert_eq!(
            nearest("totals", ["total", "tots"]),
            vec![(1, "total"), (2, "tots")]
        );
        assert_eq!(nearest("zzzzzzzz", scope), Vec::new());
        // A repeated candidate counts once, and the name itself is never offered.
        assert_eq!(nearest("cat", ["cat", "bat", "bat"]), vec![(1, "bat")]);
    }

    #[test]
    fn closest_names_are_within_a_third_of_the_length() {
        assert_eq!(closest("nme", ["name", "seats"]), Some("name"));
        assert_eq!(closest("saets", ["name", "seats"]), Some("seats"));
        assert_eq!(closest("value", ["name", "seats"]), None);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("ab", "ba"), 1);
    }
}
