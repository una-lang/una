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
mod call;
mod check;
mod effects;
mod elab;
mod env;
mod expr;
mod facts;
mod fixed;
mod flow;
mod generic;
mod index;
mod label;
mod memory;
mod records;
mod stmt;
mod summary;
mod ty;
mod unify;

pub use index::Index;
use una_diag::{Code, Diagnostic, Edit, Location, ObligationKind, Related, Repair};
use una_project::Edition;
use una_syntax::ast::File;
use una_syntax::{FileId, Interner, KeywordSet, SourceDb, Span, lex_in, parse};

use crate::names::PackageInput;
use env::{Env, Source};

/// The prelude (`spec/14 §1, §3`) as Una source.
const PRELUDE: &str = include_str!("../std/prelude.una");
/// The declarations that edition 0.2 adds to the prelude (`spec/17 §8`), appended to it.
const PRELUDE_0_2: &str = include_str!("../std/prelude_0_2.una");

/// The prelude of edition 0.2: `map`, `filter`, `first_where` and `fold` take effectful and
/// failing functions (ADR 0062, `spec/14 §1`), and `min`, `max`, `contains` and `sum` are
/// generic over constrained types (ADR 0066, `spec/20 §6`).
fn of_prelude() -> String {
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
    OF.iter().fold(PRELUDE.to_owned(), |text, (from, to)| {
        text.replace(from, to)
    })
}

/// `std.testing` (`spec/14 §4`) as Una source.
const TESTING: &str = include_str!("../std/testing.una");

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
}

/// The outcome of [`check_types`].
#[derive(Clone, Debug, Default)]
pub struct TypeCheck {
    /// The type diagnostics.
    pub diagnostics: Vec<Diagnostic>,
    /// The pending proof obligations, by position, then kind.
    pub obligations: Vec<PendingObligation>,
    /// Where each `let` whose right-hand side is in the refinement language binds its name.
    /// Such a binding has the selfified type `{v: B | v == e}` (`spec/03 §7`).
    pub selfified: Vec<Location>,
    /// The headers of the functions of the modules checked, by module, then declaration.
    pub headers: Vec<Header>,
    /// The Core IR of the declarations without diagnostics (`spec/09`).
    pub program: una_core::Program,
    /// What the checker resolved, by span, for the language server (ADR 0063).
    pub index: Index,
}

/// Type-checks the modules of `package` that have no syntax errors.
pub fn check_types(package: &PackageInput<'_>) -> TypeCheck {
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
    }
    if package.settings.edition >= Edition::V0_2 {
        summary_fixpoint(&mut env, &facts, package.sources, &edges);
    }
    authority::infer(&mut env, &collected, &mut report);
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
    facts::predicate_recursion(&env, &findings, &mut report);
    if package.settings.edition >= Edition::V0_2 {
        facts::generic_constraints(&mut env, &facts, &findings, &mut report);
    } else {
        facts::generic_equality(&env, &facts, &findings, &mut report);
    }
    authority::grants(&env, package.settings, package.run, &mut report);
    authority::budget(&env, &findings, package.settings, &mut report);
    // Pending obligations by position, then kind.
    findings
        .obligations
        .sort_by_key(|(kind, span, _)| (span.file, span.start, span.end, *kind));
    let headers = headers(&env);
    let (program, lost) = elab::program(&env, &findings, package.sources, &standard.sources);
    // ADR 0038 D1: a component of a checker type that Core IR lost is an internal error, never
    // a silent pass (`spec/10 §2`). DECISION D-144: switched to B by the owner, ADR 0045: the
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
    TypeCheck {
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
        diagnostics: report.diagnostics,
    }
}

/// The headers of the functions of the modules that report, after inference.
fn headers(env: &Env<'_>) -> Vec<Header> {
    env.functions
        .iter()
        .enumerate()
        .filter(|(_, f)| f.kind == env::FnKind::Function && env.reports(f.module))
        .map(|(id, f)| Header {
            generic: !f.type_params.is_empty(),
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
    /// Names bound by selfified `let`s.
    pub(crate) selfified: Vec<Span>,
    /// Calls of, and references to, functions and predicates, from the body of a function or
    /// predicate: caller, callee, and where.
    pub(crate) edges: Vec<(usize, usize, Span)>,
    /// Instantiations of generic functions: the caller when it is a function, the callee, its
    /// type arguments and where.
    pub(crate) instances: Vec<(Option<usize>, usize, Vec<ty::Ty>, Span)>,
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
        let (text, keywords) = if edition >= Edition::V0_2 {
            (format!("{}\n{PRELUDE_0_2}", of_prelude()), KeywordSet::V0_2)
        } else {
            (PRELUDE.to_owned(), KeywordSet::V0_1)
        };
        let prelude = sources.add("std/prelude.una", text.as_bytes());
        let testing = sources.add("std/testing.una", TESTING.as_bytes());
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
    for component in summary::components(&generic, edges) {
        loop {
            env.summaries_grew = false;
            let mut scratch = Report::new(sources);
            let mut findings = Findings::default();
            let mut collected = authority::Collected::default();
            check::functions(
                env,
                facts,
                &mut scratch,
                &mut findings,
                &mut collected,
                authority::Pass::Collect,
                &component,
            );
            if !env.summaries_grew {
                break;
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
            rank: 1,
            escape_cost: 0,
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
            rank: 1,
            escape_cost: 0,
            description: description.into(),
            edit: None,
        }
    }

    pub(crate) fn related(&self, span: Span, message: impl Into<String>) -> Related {
        Related {
            location: self.location(span),
            message: message.into(),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_names_are_within_a_third_of_the_length() {
        assert_eq!(closest("nme", ["name", "seats"]), Some("name"));
        assert_eq!(closest("saets", ["name", "seats"]), Some("seats"));
        assert_eq!(closest("value", ["name", "seats"]), None);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("ab", "ba"), 1);
    }
}
