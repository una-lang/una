//! `una report [path]` (`spec/13 §6`): the build report of a package that checks at tier 1.
//!
//! What each section lists. trivial `ensures` and coverage.
//! The report is assembled here, beside the package and analysis it reads.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};
use una_core::{Bound, Decl, Expr, Rhs};
use una_diag::{ObligationStatus, Position};
use una_syntax::Span;
use una_syntax::ast::{ClauseKind, DeclKind, TextPart};

use crate::contract::Modules;
use crate::verification::{lock_of, problem_diagnostic, report_unverified, settings, tier_one};
use crate::{Analysis, ExitStatus, Package, analyze, load, on_large_stack, report};

/// The JSON schema of `una report --json` (`spec/13 §6`).
const REPORT_SCHEMA: &str = "una.report/2";

/// The proof cache of a package, relative to its root. Its entries are authenticated under the
/// user's key, which is in the Trusted zone instead (ADR 0064).
const PROOF_CACHE: &str = ".una/cache/";

/// What the verification model does not cover: the one list of `spec/07 §1` and `spec/13 §6`,
/// in their order (R-041; the last entry is ADR 0038 D9 rule 4). `una.report/2` says "stack
/// exhaustion", since native executables exhaust their stack too (ADR 0055), and ends with the
/// order of effects across `together` tasks (ADR 0065, `spec/19 §3`), then the three entries of
/// long-running programs (ADR 0116 rule 21), then the two of ADR 0117 rule 13.
const OUT_OF_MODEL: [&str; 13] = [
    "timing",
    "termination and divergence",
    "resource consumption and allocation",
    "stack exhaustion",
    "external systems beyond declared signatures",
    "races between the check and the use of a file path",
    "hard links",
    "order of effects across concurrent tasks",
    "the time of a host stop",
    "interference of the host's load with the timing of an expression",
    "uncatchable termination",
    "waits at limits and at `start`",
    "which branch of `first of` wins, beyond its label",
];

/// The capabilities of the prelude, whose implementations the host provides (`spec/14 §3`).
const HOST_CAPABILITIES: [&str; 6] = ["Console", "Clock", "Env", "Files", "Keys", "Http"];

/// `una.report/2`, with keys in the order of `spec/13 §6`.
#[derive(Serialize)]
struct Report {
    schema: &'static str,
    package: PackageDoc,
    lock: LockDoc,
    proven: ProvenDoc,
    obligations: Counts,
    authority: Vec<AuthorityDoc>,
    handler_authority: Vec<HandlerAuthorityDoc>,
    information: InformationDoc,
    escapes: EscapesDoc,
    assumptions: Vec<AssumptionDoc>,
    tcb: TcbDoc,
    requirements: Vec<RequirementDoc>,
    coverage: CoverageDoc,
    native: NativeDoc,
    generic_results: Vec<GenericResultDoc>,
    out_of_model: [&'static str; 13],
}

/// A public generic function and its result summary (ADR 0067, `spec/13 §6`).
#[derive(Serialize)]
struct GenericResultDoc {
    function: String,
    result_labels: Vec<ResultLabelDoc>,
    of_raises: Vec<OfRaiseDoc>,
}

/// An `of` parameter of a generic function whose calls depend on decisions on type
/// parameters (ADR 0067 amendment).
#[derive(Serialize)]
struct OfRaiseDoc {
    param: String,
    from: Vec<String>,
}

/// A position of a generic function's result and the type parameters whose labels reach it.
#[derive(Serialize)]
struct ResultLabelDoc {
    position: String,
    from: Vec<String>,
}

/// The native backend (ADR 0055): the host target, and the bindings whose `Int` stays
/// arbitrary precision in native code.
#[derive(Serialize)]
struct NativeDoc {
    target: String,
    arbitrary_precision: Vec<String>,
}

#[derive(Serialize)]
struct PackageDoc {
    name: String,
    version: String,
}

#[derive(Serialize)]
struct LockDoc {
    solver_version: String,
    hash: String,
}

#[derive(Serialize)]
struct ProvenDoc {
    semantic: usize,
    security: &'static str,
    authority: &'static str,
}

#[derive(Default, Serialize)]
struct Counts {
    proven: usize,
    refuted: usize,
    unknown: usize,
    /// How many of these results came from the proof cache (ADR 0041).
    cached: usize,
}

#[derive(Serialize)]
struct AuthorityDoc {
    function: String,
    public: bool,
    uses: Vec<String>,
    inferred: bool,
}

/// A handler with `uses` clauses (ADR 0081, #14): additive under ADR 0076.
#[derive(Serialize)]
struct HandlerAuthorityDoc {
    handler: String,
    capability: String,
    public: bool,
    uses: Vec<String>,
}

#[derive(Serialize)]
struct InformationDoc {
    policy_calls: Vec<PolicyCallDoc>,
    /// Each site whose deadline turns timing into values, with its timing label (ADR 0116
    /// rule 21); absent when there is none, as in every package before edition 0.4.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    timing: Vec<TimingDoc>,
}

/// A site with a deadline and its timing label (ADR 0116 rule 21).
#[derive(Serialize)]
struct TimingDoc {
    construct: &'static str,
    function: String,
    label: String,
    site: SiteDoc,
}

#[derive(Serialize)]
struct PolicyCallDoc {
    policy: String,
    purpose: String,
    owner: String,
    site: SiteDoc,
}

#[derive(Serialize)]
struct SiteDoc {
    file: String,
    start: Position,
}

#[derive(Serialize)]
struct EscapesDoc {
    checked: Vec<EscapeDoc>,
}

#[derive(Serialize)]
struct EscapeDoc {
    module: String,
    count: usize,
    budget: u64,
}

#[derive(Serialize)]
struct AssumptionDoc {
    capability: String,
    operations: Vec<String>,
    ensures: Vec<String>,
}

#[derive(Serialize)]
struct TcbDoc {
    trusted_modules: Vec<TrustedDoc>,
    host_capabilities: Vec<String>,
    solver: String,
    /// The proof cache of the package.
    proof_cache: &'static str,
    /// The key file that authenticates the cache's entries, part of the Trusted zone, or `null`
    /// when the run used no cache (ADR 0064).
    proof_cache_key: Option<String>,
}

#[derive(Serialize)]
struct TrustedDoc {
    module: String,
    lines: usize,
}

#[derive(Serialize)]
struct RequirementDoc {
    id: String,
    status: &'static str,
    derived: Vec<String>,
}

#[derive(Serialize)]
struct CoverageDoc {
    pub_functions: usize,
    with_ensures: usize,
    with_derived_from: usize,
    trivial_ensures: usize,
}

/// The options of `una report`.
#[derive(Clone, Debug)]
pub(crate) struct ReportOptions {
    pub(crate) json: bool,
    pub(crate) color: bool,
    /// How tier 1 runs the solver (ADR 0064).
    pub(crate) solving: crate::verification::Solving,
}

/// `una report [path]`: checks the package at tier 1 and prints its build report; the
/// diagnostics of tier 0, or of a solver that cannot run, instead of it.
pub(crate) fn report_command(
    path: &Path,
    options: ReportOptions,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    let package = match load(path) {
        Ok(package) => package,
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let analysis = on_large_stack(|| analyze(&package, None));
    if !analysis.diagnostics.is_empty() {
        return report(
            &analysis.diagnostics,
            &package.sources,
            options.json,
            options.color,
            stdout,
            stderr,
        );
    }
    let found = tier_one(&package, &analysis.program, &options.solving);
    let Some(verification) = found.verification else {
        return report_unverified(
            &found.diagnostics,
            &package,
            options.json,
            options.color,
            stdout,
            stderr,
        );
    };
    // A clause the solver proves without any hypothesis is trivial.
    let trivial_queries =
        on_large_stack(|| una_verify::trivial_ensures(&analysis.program, &package.sources));
    let trivial: BTreeSet<(String, Span)> =
        match una_verify::verify(&trivial_queries, &settings(&package, &options.solving)) {
            Ok(outcomes) => trivial_queries
                .iter()
                .zip(outcomes)
                .filter(|(_, outcome)| outcome.status == ObligationStatus::Proven)
                .map(|(obligation, _)| (obligation.declaration.clone(), obligation.source))
                .collect(),
            Err(problem) => {
                return report_unverified(
                    &[problem_diagnostic(&package, problem)],
                    &package,
                    options.json,
                    options.color,
                    stdout,
                    stderr,
                );
            }
        };
    let mut counts = Counts::default();
    for outcome in &verification.outcomes {
        match outcome.status {
            ObligationStatus::Proven => counts.proven += 1,
            ObligationStatus::Refuted => counts.refuted += 1,
            ObligationStatus::Unknown => counts.unknown += 1,
        }
        if outcome.cached {
            counts.cached += 1;
        }
    }
    let unproven = counts.refuted + counts.unknown > 0;
    // The key appears when this run used a cache, which only a package with a key has.
    let key = settings(&package, &options.solving)
        .cache
        .and(options.solving.proof_key.as_ref())
        .map(|key| key.shown.clone());
    let document = build(&package, &analysis, counts, &trivial, key);
    if options.json {
        match serde_json::to_string(&document) {
            Ok(text) => writeln!(stdout, "{text}")?,
            Err(error) => {
                writeln!(
                    stderr,
                    "internal compiler error (U8001): rendering the report as JSON failed: {error}"
                )?;
                return Ok(ExitStatus::Internal);
            }
        }
    } else {
        write!(stdout, "{}", human(&document))?;
        if !found.diagnostics.is_empty() {
            write!(
                stderr,
                "{}",
                una_diag::render_human(&found.diagnostics, &package.sources.texts(), options.color)
            )?;
        }
    }
    stdout.flush()?;
    Ok(if unproven {
        ExitStatus::Errors
    } else {
        ExitStatus::Success
    })
}

fn build(
    package: &Package,
    analysis: &Analysis,
    counts: Counts,
    trivial: &BTreeSet<(String, Span)>,
    proof_cache_key: Option<String>,
) -> Report {
    let (manifest, _) = crate::package::project(package);
    let modules = Modules {
        sources: &package.sources,
        interner: &analysis.interner,
        trees: &analysis.trees,
        mounts: &analysis.mounts,
    };
    let program = &analysis.program;
    let (lock_text, lock) = lock_of(package);
    // The digest of the lock file as written; the key text of the default lock otherwise.
    let lock_bytes = package
        .lock
        .map_or(lock_text.as_str(), |id| package.sources.file(id).text());
    let hash: String = Sha256::digest(lock_bytes.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let package_doc = PackageDoc {
        name: manifest.name.as_ref().map_or_else(
            || analysis.entry.clone().unwrap_or_default(),
            |name| name.value.clone(),
        ),
        version: manifest
            .version
            .as_ref()
            .map_or_else(|| "0.0.0".to_owned(), |version| version.value.clone()),
    };
    let public: BTreeMap<String, bool> = program
        .modules
        .iter()
        .flat_map(|module| {
            module.decls.iter().filter_map(|decl| match decl {
                Decl::Fun(fun) => Some((format!("{}.{}", module.path, fun.name), fun.public)),
                _ => None,
            })
        })
        .collect();
    let declared_capabilities: BTreeSet<&str> = program
        .modules
        .iter()
        .flat_map(|module| {
            module.decls.iter().filter_map(|decl| match decl {
                Decl::Cap { name, .. } => Some(name.as_str()),
                _ => None,
            })
        })
        .collect();
    // ADR 0067: the result summaries of the public generic functions.
    let generic_results: Vec<GenericResultDoc> = analysis
        .headers
        .iter()
        .filter(|header| header.generic)
        .map(|header| (format!("{}.{}", header.module, header.name), header))
        .filter(|(function, _)| public.get(function).copied().unwrap_or(false))
        .map(|(function, header)| GenericResultDoc {
            function,
            result_labels: header
                .result_labels
                .iter()
                .map(|(position, from)| ResultLabelDoc {
                    position: position.clone(),
                    from: from.clone(),
                })
                .collect(),
            of_raises: header
                .of_raises
                .iter()
                .map(|(param, from)| OfRaiseDoc {
                    param: param.clone(),
                    from: from.clone(),
                })
                .collect(),
        })
        .collect();
    let mut authority = Vec::new();
    let mut host = BTreeSet::new();
    for header in &analysis.headers {
        if header.uses.is_empty() && header.uses_of.is_empty() {
            continue;
        }
        let function = format!("{}.{}", header.module, header.name);
        for used in &header.uses {
            if HOST_CAPABILITIES.contains(&used.capability.as_str())
                && !declared_capabilities.contains(used.capability.as_str())
            {
                host.insert(used.capability.clone());
            }
        }
        authority.push(AuthorityDoc {
            public: public.get(&function).copied().unwrap_or(false),
            function,
            // ADR 0062: `uses of p` is listed as written, `of p`.
            uses: header
                .uses
                .iter()
                .map(|u| format!("@{}: {}[{}]", u.cap, u.capability, u.ops.join(", ")))
                .chain(header.uses_of.iter().map(|param| format!("of {param}")))
                .collect(),
            inferred: header.uses_inferred,
        });
    }
    authority.sort_by(|a, b| a.function.cmp(&b.function));
    // ADR 0081 (#14): every handler with `uses`, whose host capabilities count for the TCB too.
    let public_handlers = public_handlers(&modules);
    let mut handler_authority = Vec::new();
    for module in &program.modules {
        for decl in &module.decls {
            let Decl::Handler {
                name,
                capability_path,
                uses,
                ..
            } = decl
            else {
                continue;
            };
            if uses.is_empty() {
                continue;
            }
            for used in uses {
                if HOST_CAPABILITIES.contains(&used.capability.as_str())
                    && !declared_capabilities.contains(used.capability.as_str())
                {
                    host.insert(used.capability.clone());
                }
            }
            let handler = format!("{}.{name}", module.path);
            handler_authority.push(HandlerAuthorityDoc {
                public: public_handlers.contains(&handler),
                handler,
                capability: capability_path.qualified(),
                uses: uses
                    .iter()
                    .map(|u| format!("@{}: {}[{}]", u.cap, u.capability, u.ops.join(", ")))
                    .collect(),
            });
        }
    }
    handler_authority.sort_by(|a, b| a.handler.cmp(&b.handler));
    // Policy calls and `checked`, from Core IR.
    let mut calls: Vec<(Span, String)> = Vec::new();
    let mut checked: BTreeMap<String, usize> = BTreeMap::new();
    for module in &program.modules {
        let mut sites = Sites::default();
        for decl in &module.decls {
            sites.decl(decl);
        }
        for example in &module.examples {
            sites.expr(&example.body);
        }
        if sites.checked > 0 {
            checked.insert(module.path.clone(), sites.checked);
        }
        calls.extend(sites.declassify);
    }
    calls.sort();
    let policies = policies(&modules);
    let policy_calls = calls
        .into_iter()
        .map(|(span, policy)| {
            let (purpose, owner) = policies.get(&policy).cloned().unwrap_or_default();
            let location = package.sources.location(span);
            PolicyCallDoc {
                policy,
                purpose,
                owner,
                site: SiteDoc {
                    file: location.file,
                    start: location.start,
                },
            }
        })
        .collect();
    let escapes = EscapesDoc {
        checked: checked
            .into_iter()
            .map(|(module, count)| EscapeDoc {
                module,
                count,
                budget: manifest.checked_per_module,
            })
            .collect(),
    };
    let mut assumptions = Vec::new();
    for module in &program.modules {
        for decl in &module.decls {
            if let Decl::Cap { name, ops } = decl {
                assumptions.push(AssumptionDoc {
                    capability: format!("{}.{name}", module.path),
                    operations: ops.iter().map(|op| op.name.clone()).collect(),
                    ensures: ops
                        .iter()
                        .flat_map(|op| {
                            op.ensures
                                .iter()
                                .map(move |clause| format!("{}: {}", op.name, clause.text))
                        })
                        .collect(),
                });
            }
        }
    }
    assumptions.sort_by(|a, b| a.capability.cmp(&b.capability));
    let mut trusted_modules: Vec<TrustedDoc> = manifest
        .trusted
        .iter()
        .map(|trusted| TrustedDoc {
            module: trusted.value.clone(),
            lines: module_lines(&modules, &trusted.value),
        })
        .collect();
    trusted_modules.sort_by(|a, b| a.module.cmp(&b.module));
    let (requirements, derived_functions) = requirements(&modules);
    let public_functions: Vec<(&String, &una_core::Fun)> = program
        .modules
        .iter()
        .flat_map(|module| {
            module.decls.iter().filter_map(move |decl| match decl {
                Decl::Fun(fun) if fun.public => Some((&module.path, fun.as_ref())),
                _ => None,
            })
        })
        .collect();
    let mut with_ensures = 0;
    let mut with_derived_from = 0;
    let mut trivial_ensures = 0;
    for (module, fun) in &public_functions {
        let qualified = format!("{module}.{}", fun.name);
        let trivial_here = fun
            .ensures
            .iter()
            .filter(|clause| {
                trivial.contains(&(qualified.clone(), clause.span.unwrap_or(fun.span)))
            })
            .count();
        trivial_ensures += trivial_here;
        if fun.ensures.len() > trivial_here {
            with_ensures += 1;
        }
        if derived_functions.contains(&qualified) {
            with_derived_from += 1;
        }
    }
    Report {
        schema: REPORT_SCHEMA,
        package: package_doc,
        lock: LockDoc {
            solver_version: lock.solver_version.clone(),
            hash,
        },
        proven: ProvenDoc {
            semantic: counts.proven,
            security: "by typing",
            authority: "by typing",
        },
        obligations: counts,
        authority,
        handler_authority,
        information: InformationDoc {
            policy_calls,
            timing: analysis
                .timing
                .iter()
                .map(|site| TimingDoc {
                    construct: site.construct,
                    function: site.function.clone(),
                    label: site.label.to_string(),
                    site: SiteDoc {
                        file: site.location.file.clone(),
                        start: site.location.start,
                    },
                })
                .collect(),
        },
        escapes,
        assumptions,
        tcb: TcbDoc {
            trusted_modules,
            host_capabilities: host.into_iter().collect(),
            solver: format!("z3 {}", lock.solver_version),
            proof_cache: PROOF_CACHE,
            proof_cache_key,
        },
        requirements,
        coverage: CoverageDoc {
            pub_functions: public_functions.len(),
            with_ensures,
            with_derived_from,
            trivial_ensures,
        },
        native: NativeDoc {
            target: una_codegen::host_target(),
            arbitrary_precision: arbitrary_precision(program),
        },
        generic_results,
        out_of_model: OUT_OF_MODEL,
    }
}

/// The parameters and results of the package's functions whose type is `Int` with no proven
/// range that fits a machine word, as `module.function.parameter` and `module.function.result`:
/// the native backend keeps them at arbitrary precision (ADR 0055 NB-3, ADR 0057 FW-1). A
/// parameter's refinements are proven by each call, and a result's by the function.
fn arbitrary_precision(program: &una_core::Program) -> Vec<String> {
    let int = |ty: &una_core::Type| {
        matches!(
            ty,
            una_core::Type::Base {
                base: una_core::Base::Int,
                ..
            }
        ) && !una_codegen::machine_width(ty)
    };
    let mut bindings = Vec::new();
    for module in &program.modules {
        for decl in &module.decls {
            let una_core::Decl::Fun(fun) = decl else {
                continue;
            };
            let qualified = format!("{}.{}", module.path, fun.name);
            for (name, ty) in &fun.params {
                if int(ty) {
                    bindings.push(format!("{qualified}.{name}"));
                }
            }
            if int(&fun.result) {
                bindings.push(format!("{qualified}.result"));
            }
        }
    }
    bindings
}

/// The policy call sites and `checked` escapes of Core IR.
#[derive(Default)]
struct Sites {
    declassify: Vec<(Span, String)>,
    checked: usize,
}

impl Sites {
    fn decl(&mut self, decl: &Decl) {
        match decl {
            Decl::Fun(fun) => self.expr(&fun.body),
            Decl::Pred { body, .. } => self.expr(body),
            Decl::Handler { ops, .. } => {
                for op in ops {
                    self.expr(&op.body);
                }
            }
            _ => {}
        }
    }

    fn expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Let { value, body, .. } => {
                match value.as_ref() {
                    Bound::Rhs(rhs) => self.rhs(rhs),
                    Bound::Expr(inner) => self.expr(inner),
                }
                self.expr(body);
            }
            Expr::LetTuple { body, .. } | Expr::Loop { body, .. } => self.expr(body),
            Expr::Together { tasks, .. } | Expr::FirstOf { tasks, .. } => {
                for task in tasks {
                    self.expr(task);
                }
            }
            Expr::TogetherFor { task, .. } => self.expr(task),
            Expr::If {
                then, otherwise, ..
            } => {
                self.expr(then);
                self.expr(otherwise);
            }
            Expr::Case {
                arms, otherwise, ..
            } => {
                for arm in arms {
                    self.expr(&arm.body);
                }
                if let Some(otherwise) = otherwise {
                    self.expr(otherwise);
                }
            }
            Expr::Handle { body, handler, .. } => {
                self.expr(body);
                self.expr(handler);
            }
            Expr::Attempt { body, .. }
            | Expr::With { body, .. }
            | Expr::Within { body, .. }
            | Expr::Tasks { body, .. } => {
                self.expr(body);
            }
            Expr::Start { task, .. } => self.expr(task),
            Expr::Atom { .. }
            | Expr::Fail { .. }
            | Expr::Exit { .. }
            | Expr::Unreachable { .. } => {}
        }
    }

    fn rhs(&mut self, rhs: &Rhs) {
        match rhs {
            Rhs::Declassify { policy, span, .. } => {
                self.declassify.push((*span, policy.qualified()));
            }
            Rhs::Checked { predicate, .. } => {
                self.checked += 1;
                self.expr(predicate);
            }
            Rhs::Lambda { body, .. } => self.expr(body),
            _ => {}
        }
    }
}

/// The purpose and owner of every policy, by qualified name.
/// The `pub` handlers of the package, by qualified name.
fn public_handlers(modules: &Modules<'_>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (_, tree) in modules.trees {
        let Some(path) = modules.path(tree) else {
            continue;
        };
        for decl in &tree.decls {
            if let DeclKind::Handler(handler) = &decl.kind
                && handler.public
            {
                out.insert(format!("{path}.{}", modules.name(handler.name.name)));
            }
        }
    }
    out
}

fn policies(modules: &Modules<'_>) -> BTreeMap<String, (String, String)> {
    let mut out = BTreeMap::new();
    for (_, tree) in modules.trees {
        let Some(path) = modules.path(tree) else {
            continue;
        };
        for decl in &tree.decls {
            if let DeclKind::Policy(policy) = &decl.kind {
                let purpose: String = policy
                    .purpose
                    .parts
                    .iter()
                    .map(|part| match part {
                        TextPart::Text(text) => text.clone(),
                        TextPart::Interpolation(expr) => format!("{{{}}}", modules.text(expr.span)),
                    })
                    .collect();
                out.insert(
                    format!("{path}.{}", modules.name(policy.name.name)),
                    (purpose, modules.name(policy.owner.name).to_owned()),
                );
            }
        }
    }
    out
}

/// The requirements by identifier, with the functions derived from each, and every function
/// with a `derived from` clause.
fn requirements(modules: &Modules<'_>) -> (Vec<RequirementDoc>, BTreeSet<String>) {
    let mut declared: BTreeMap<String, bool> = BTreeMap::new();
    let mut derived: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut functions = BTreeSet::new();
    for (_, tree) in modules.trees {
        let Some(path) = modules.path(tree) else {
            continue;
        };
        for decl in &tree.decls {
            match &decl.kind {
                DeclKind::Requirement(requirement) => {
                    declared.insert(
                        modules.name(requirement.id.name).to_owned(),
                        requirement.active,
                    );
                }
                DeclKind::Function(function) => {
                    let qualified = format!("{path}.{}", modules.name(function.name.name));
                    for clause in &function.clauses {
                        if let ClauseKind::DerivedFrom(ids) = &clause.kind {
                            functions.insert(qualified.clone());
                            for id in ids {
                                derived
                                    .entry(modules.name(id.name).to_owned())
                                    .or_default()
                                    .insert(qualified.clone());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let docs = declared
        .into_iter()
        .map(|(id, active)| RequirementDoc {
            status: if active { "active" } else { "proposed" },
            derived: derived
                .get(&id)
                .map(|set| set.iter().cloned().collect())
                .unwrap_or_default(),
            id,
        })
        .collect();
    (docs, functions)
}

/// The number of lines of the file of module `module`.
fn module_lines(modules: &Modules<'_>, module: &str) -> usize {
    modules
        .trees
        .iter()
        .find(|(_, tree)| modules.path(tree).as_deref() == Some(module))
        .map_or(0, |(id, _)| {
            modules.sources.file(*id).text().lines().count()
        })
}

/// The report as labeled lines.
fn human(report: &Report) -> String {
    let mut out = String::new();
    let mut line = |text: String| {
        out.push_str(&text);
        out.push('\n');
    };
    line(format!(
        "package {} {}",
        report.package.name, report.package.version
    ));
    line(format!(
        "lock: z3 {}, sha256 {}",
        report.lock.solver_version, report.lock.hash
    ));
    line(format!(
        "proven: {} semantic obligations; security and authority by typing",
        report.proven.semantic
    ));
    line(format!(
        "obligations: {} proven, {} refuted, {} unknown; {} cached",
        report.obligations.proven,
        report.obligations.refuted,
        report.obligations.unknown,
        report.obligations.cached
    ));
    line("authority:".to_owned());
    for entry in &report.authority {
        let visibility = if entry.public { "pub" } else { "private" };
        let inferred = if entry.inferred { ", inferred" } else { "" };
        line(format!(
            "  {} ({visibility}{inferred}): {}",
            entry.function,
            entry.uses.join(", ")
        ));
    }
    line("handler authority:".to_owned());
    for entry in &report.handler_authority {
        let visibility = if entry.public { "pub" } else { "private" };
        line(format!(
            "  {} for {} ({visibility}): {}",
            entry.handler,
            entry.capability,
            entry.uses.join(", ")
        ));
    }
    line("information:".to_owned());
    for call in &report.information.policy_calls {
        line(format!(
            "  {} at {}:{}:{}, purpose {}, owner {}",
            call.policy,
            call.site.file,
            call.site.start.line,
            call.site.start.col,
            una_core::quote(&call.purpose),
            call.owner
        ));
    }
    for site in &report.information.timing {
        line(format!(
            "  timing of {} in {} at {}:{}:{}: {}",
            site.construct,
            site.function,
            site.site.file,
            site.site.start.line,
            site.site.start.col,
            site.label
        ));
    }
    line("escapes:".to_owned());
    for escape in &report.escapes.checked {
        line(format!(
            "  {}: {} checked, budget {}",
            escape.module, escape.count, escape.budget
        ));
    }
    line("assumptions:".to_owned());
    for assumption in &report.assumptions {
        let ensures = if assumption.ensures.is_empty() {
            String::new()
        } else {
            format!("; ensures {}", assumption.ensures.join("; "))
        };
        line(format!(
            "  {}: {}{ensures}",
            assumption.capability,
            assumption.operations.join(", ")
        ));
    }
    line("tcb:".to_owned());
    let trusted: Vec<String> = report
        .tcb
        .trusted_modules
        .iter()
        .map(|t| format!("{} ({} lines)", t.module, t.lines))
        .collect();
    line(format!("  trusted modules: {}", trusted.join(", ")));
    line(format!(
        "  host capabilities: {}",
        report.tcb.host_capabilities.join(", ")
    ));
    line(format!("  solver: {}", report.tcb.solver));
    line(format!("  proof cache: {}", report.tcb.proof_cache));
    line(format!(
        "  proof cache key: {}",
        report
            .tcb
            .proof_cache_key
            .as_deref()
            .unwrap_or("none, so the cache is off")
    ));
    line("requirements:".to_owned());
    for requirement in &report.requirements {
        line(format!(
            "  {} ({}): {}",
            requirement.id,
            requirement.status,
            requirement.derived.join(", ")
        ));
    }
    let coverage = &report.coverage;
    line(format!(
        "coverage: {} pub functions, {} with ensures, {} with derived from, {} trivial ensures",
        coverage.pub_functions,
        coverage.with_ensures,
        coverage.with_derived_from,
        coverage.trivial_ensures
    ));
    line(format!("native: {}", report.native.target));
    line(format!(
        "  arbitrary precision: {}",
        report.native.arbitrary_precision.join(", ")
    ));
    for generic in &report.generic_results {
        let parts: Vec<String> = generic
            .result_labels
            .iter()
            .map(|label| format!("{} from {}", label.position, label.from.join(", ")))
            .collect();
        let mut parts = parts;
        parts.extend(
            generic
                .of_raises
                .iter()
                .map(|raise| format!("{} runs under {}", raise.param, raise.from.join(", "))),
        );
        let summary = if parts.is_empty() {
            "none".to_owned()
        } else {
            parts.join("; ")
        };
        line(format!("generic result {}: {summary}", generic.function));
    }
    line(format!("out of model: {}", report.out_of_model.join(", ")));
    out
}
