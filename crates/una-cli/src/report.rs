//! `una report [path]` (`spec/13 §6`): the build report of a package that checks at tier 1.
//!
//! DECISION D-122: what each section lists. DECISION D-123: trivial `ensures` and coverage.
//! DECISION D-132: the report is assembled here, beside the package and analysis it reads.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

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
const REPORT_SCHEMA: &str = "una.report/1";

/// What the verification model does not cover (`spec/13 §6`, `spec/07 §1`).
const OUT_OF_MODEL: [&str; 4] = [
    "timing",
    "termination",
    "resource consumption",
    "external systems beyond declared signatures",
];

/// The capabilities of the prelude, whose implementations the host provides (`spec/14 §3`).
const HOST_CAPABILITIES: [&str; 5] = ["Console", "Clock", "Env", "Files", "Keys"];

/// `una.report/1`, with keys in the order of `spec/13 §6`.
#[derive(Serialize)]
struct Report {
    schema: &'static str,
    package: PackageDoc,
    lock: LockDoc,
    proven: ProvenDoc,
    obligations: Counts,
    authority: Vec<AuthorityDoc>,
    information: InformationDoc,
    escapes: EscapesDoc,
    assumptions: Vec<AssumptionDoc>,
    tcb: TcbDoc,
    requirements: Vec<RequirementDoc>,
    coverage: CoverageDoc,
    out_of_model: [&'static str; 4],
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
}

#[derive(Serialize)]
struct AuthorityDoc {
    function: String,
    public: bool,
    uses: Vec<String>,
    inferred: bool,
}

#[derive(Serialize)]
struct InformationDoc {
    policy_calls: Vec<PolicyCallDoc>,
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
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReportOptions {
    pub(crate) json: bool,
    pub(crate) color: bool,
    pub(crate) wallclock: Duration,
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
    let found = tier_one(&package, &analysis.program, options.wallclock);
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
    // DECISION D-123: a clause the solver proves without any hypothesis is trivial.
    let trivial_queries =
        on_large_stack(|| una_verify::trivial_ensures(&analysis.program, &package.sources));
    let trivial: BTreeSet<(String, Span)> =
        match una_verify::verify(&trivial_queries, &settings(&package, options.wallclock)) {
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
    }
    let unproven = counts.refuted + counts.unknown > 0;
    let document = build(&package, &analysis, counts, &trivial);
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
) -> Report {
    let (manifest, _) = crate::package::project(package);
    let modules = Modules {
        sources: &package.sources,
        interner: &analysis.interner,
        trees: &analysis.trees,
    };
    let program = &analysis.program;
    let (lock_text, lock) = lock_of(package);
    let hash: String = Sha256::digest(lock_text.as_bytes())
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
    let mut authority = Vec::new();
    let mut host = BTreeSet::new();
    for header in &analysis.headers {
        if header.uses.is_empty() {
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
            uses: header
                .uses
                .iter()
                .map(|u| format!("@{}: {}[{}]", u.cap, u.capability, u.ops.join(", ")))
                .collect(),
            inferred: header.uses_inferred,
        });
    }
    authority.sort_by(|a, b| a.function.cmp(&b.function));
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
                Decl::Fun(fun) if fun.public => Some((&module.path, fun)),
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
        information: InformationDoc { policy_calls },
        escapes,
        assumptions,
        tcb: TcbDoc {
            trusted_modules,
            host_capabilities: host.into_iter().collect(),
            solver: format!("z3 {}", lock.solver_version),
        },
        requirements,
        coverage: CoverageDoc {
            pub_functions: public_functions.len(),
            with_ensures,
            with_derived_from,
            trivial_ensures,
        },
        out_of_model: OUT_OF_MODEL,
    }
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
            Expr::Attempt { body, .. } | Expr::With { body, .. } => self.expr(body),
            Expr::Atom { .. } | Expr::Fail { .. } | Expr::Unreachable { .. } => {}
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
        "obligations: {} proven, {} refuted, {} unknown",
        report.obligations.proven, report.obligations.refuted, report.obligations.unknown
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
    line(format!("out of model: {}", report.out_of_model.join(", ")));
    out
}
