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
use crate::{Analysis, ExitStatus, Package, analyze, on_large_stack, report};

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
    /// Every package of the program but the root, in name order (ADR 0122 rule 58).
    dependencies: Vec<DependencyDoc>,
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

/// The native backend (ADR 0055): the host target, the bindings whose `Int` stays
/// arbitrary precision in native code, the dense kernels' variants (ADR 0120), and the digest
/// of the runtime built into `una`, in a release build (ADR 0124 rule 28).
#[derive(Serialize)]
struct NativeDoc {
    target: String,
    arbitrary_precision: Vec<String>,
    kernels: KernelsDoc,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_sha256: Option<String>,
}

/// The variants of the dense kernels that an executable for the host target holds and chooses
/// among at start (ADR 0120 rules 56 and 57), and the allowlist of the FMA disassembly gate:
/// the demangled prefixes of the symbols where an x86-64 executable may hold an FMA
/// instruction (ADR 0120 Appendix A, the amendment of ADR 0106 item 3).
#[derive(Serialize)]
struct KernelsDoc {
    variants: Vec<String>,
    fma_allowlist: Vec<String>,
}

#[derive(Serialize)]
struct PackageDoc {
    name: String,
    version: String,
    /// `app` or `systems` (ADR 0109 rule 7).
    profile: String,
    /// The declared memory budget in MiB, present only when the manifest declares one (ADR
    /// 0109 rule 4).
    #[serde(skip_serializing_if = "Option::is_none")]
    max_memory: Option<u64>,
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

/// A dependency of the program (ADR 0122 rule 58): where it comes from, what each dependent
/// grants it, what it can ever exercise, what it uses, and the trust it adds.
#[derive(Serialize)]
pub(crate) struct DependencyDoc {
    package: String,
    version: String,
    local: String,
    source: String,
    path: Option<String>,
    digest: Option<String>,
    signed: Option<String>,
    replaced: bool,
    edition: &'static str,
    profile: String,
    granted_by: Vec<GrantedByDoc>,
    effective_ceiling: Vec<String>,
    authority: Vec<String>,
    trust: DependencyTrustDoc,
}

impl DependencyDoc {
    /// Its name, its directory when it is a path package, and what its code uses: the union of
    /// the `uses` of its functions and handlers, as `Cap[op, op]` entries.
    pub(crate) fn uses(&self) -> (&str, Option<&str>, &[String]) {
        (&self.package, self.path.as_deref(), &self.authority)
    }
}

/// The ceiling one dependent writes for a dependency; `source` is `null` for the root.
#[derive(Serialize)]
struct GrantedByDoc {
    package: String,
    source: Option<String>,
    may: Vec<String>,
}

/// The trusted modules of a dependency that the root consented to.
#[derive(Serialize)]
struct DependencyTrustDoc {
    modules: Vec<String>,
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
    /// Each registry source of the program's mounted releases, with the digest of its root
    /// metadata that the lock records (ADR 0122 rule 58).
    registries: Vec<RegistryDoc>,
    /// The trusted modules of dependencies that the root consented to, with their line counts,
    /// by package: none while trust lines do not load.
    dependency_trust: Vec<DependencyTrustedDoc>,
    /// The Z3 built into `una`: the digests of its source tarball and of its library (ADR 0124
    /// rules 20 and 28); absent in a development build without the solver.
    #[serde(skip_serializing_if = "Option::is_none")]
    solver_build: Option<SolverBuildDoc>,
    /// How the solver children were confined, `own` or `outer` (ADR 0124 rule 7); absent when
    /// no child ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    solver_confinement: Option<String>,
    /// Under an outer sandbox, the operations it denies, in rule 7's order.
    #[serde(skip_serializing_if = "Option::is_none")]
    solver_outer_denied: Option<Vec<String>>,
}

/// The digests of the solver's build (ADR 0124 rule 20).
#[derive(Serialize)]
struct SolverBuildDoc {
    /// The SHA-256 of Z3's source tarball.
    source_sha256: String,
    /// The SHA-256 of the library linked into `una`.
    library_sha256: String,
}

/// A registry source and the digest of its root metadata.
#[derive(Serialize)]
struct RegistryDoc {
    source: String,
    root: String,
}

/// A dependency's trusted modules, by package.
#[derive(Serialize)]
struct DependencyTrustedDoc {
    package: String,
    modules: Vec<TrustedDoc>,
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
    let package = match crate::package::load_in(
        path,
        &std::collections::BTreeMap::new(),
        options.solving.itaoca.as_ref(),
    ) {
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
    let layout = crate::verification::source_layout(&package);
    let trivial_queries =
        on_large_stack(|| una_verify::trivial_ensures(&analysis.program, &package.sources, layout));
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
        profile: manifest.profile.name().to_owned(),
        max_memory: manifest.max_memory,
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
    // The root's own trusted modules; those of dependencies are under `dependency_trust`.
    let mut trusted_modules: Vec<TrustedDoc> = manifest
        .trusted
        .iter()
        .filter(|trusted| !manifest.dependency_trusted.contains(&trusted.value))
        .map(|trusted| TrustedDoc {
            module: trusted.value.clone(),
            lines: module_lines(&modules, &trusted.value),
        })
        .collect();
    trusted_modules.sort_by(|a, b| a.module.cmp(&b.module));
    // The trusted modules of dependencies the root consented to, by package (ADR 0122 rule 58).
    let mut dependency_trust: Vec<DependencyTrustedDoc> = Vec::new();
    for mount in &manifest.mounts {
        let prefix = format!("{}.", mount.local);
        let mut trusted: Vec<TrustedDoc> = manifest
            .dependency_trusted
            .iter()
            .filter(|path| path.starts_with(&prefix))
            .map(|path| TrustedDoc {
                module: path.clone(),
                lines: module_lines(&modules, path),
            })
            .collect();
        if trusted.is_empty() {
            continue;
        }
        trusted.sort_by(|a, b| a.module.cmp(&b.module));
        let name = package
            .dependencies
            .iter()
            .find(|d| mount.src == format!("{}/src/", d.dir))
            .and_then(|d| manifest.ita.as_deref()?.needs.get(d.need))
            .map_or_else(|| mount.local.clone(), |need| need.name.value.to_string());
        dependency_trust.push(DependencyTrustedDoc {
            package: name,
            modules: trusted,
        });
    }
    dependency_trust.sort_by(|a, b| a.package.cmp(&b.package));
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
        dependencies: dependencies(package, &manifest, analysis),
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
            registries: registries_of(package, &manifest),
            dependency_trust,
            solver_build: una_z3::BUILD.linked.then(|| SolverBuildDoc {
                source_sha256: una_z3::BUILD.source_sha256.to_owned(),
                library_sha256: una_z3::BUILD.library_sha256.to_owned(),
            }),
            solver_confinement: crate::solver_child::confinement_seen().map(|(kind, _)| kind),
            solver_outer_denied: crate::solver_child::confinement_seen()
                .filter(|(kind, _)| kind == "outer")
                .map(|(_, denied)| denied),
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
            kernels: KernelsDoc {
                variants: una_core::dense::variants::variant_names()
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect(),
                fma_allowlist: una_core::dense::variants::ALLOWLIST
                    .iter()
                    .map(|prefix| (*prefix).to_owned())
                    .collect(),
            },
            runtime_sha256: crate::runtime_archive::SHA256.map(str::to_owned),
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
    line(format!("profile: {}", report.package.profile));
    if let Some(max_memory) = report.package.max_memory {
        line(format!("max memory: {max_memory} MiB"));
    }
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
    if !report.dependencies.is_empty() {
        line("dependencies:".to_owned());
        for dependency in &report.dependencies {
            line(format!("  {}", dependency_line(dependency)));
        }
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
    if let Some(build) = &report.tcb.solver_build {
        line(format!(
            "  solver build: source {}, library {}",
            build.source_sha256, build.library_sha256
        ));
    }
    if let Some(confinement) = &report.tcb.solver_confinement {
        line(format!("  solver confinement: {confinement}"));
    }
    if let Some(denied) = &report.tcb.solver_outer_denied {
        line(format!(
            "  solver outer sandbox denies: {}",
            denied.join(", ")
        ));
    }
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
    line(format!(
        "  kernels: {}",
        report.native.kernels.variants.join(", ")
    ));
    line(format!(
        "  fma allowlist: {}",
        report.native.kernels.fma_allowlist.join(", ")
    ));
    if let Some(runtime) = &report.native.runtime_sha256 {
        line(format!("  runtime sha256: {runtime}"));
    }
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

/// The `dependencies` of the report (ADR 0122 rule 58): each mounted path dependency, its
/// ceiling as the root writes it, its effective ceiling (rule 51), and the union of the `uses`
/// of its functions and handlers. O(H + P log P) for H headers and P dependencies.
pub(crate) fn dependencies(
    package: &Package,
    manifest: &una_project::Manifest,
    analysis: &crate::Analysis,
) -> Vec<DependencyDoc> {
    let Some(read) = manifest.ita.as_deref() else {
        return Vec::new();
    };
    let root = read
        .name
        .as_ref()
        .map(|name| name.value.to_string())
        .unwrap_or_default();
    // The lock, for a registry release's line and its key.
    let lock = package
        .lock
        .map(|id| una_project::ita::parse_ita_lock(package.sources.file(id).text()).0);
    let mut out = Vec::new();
    for mount in &manifest.mounts {
        let Some(dependency) = package
            .dependencies
            .iter()
            .find(|d| mount.src == format!("{}/src/", d.dir))
        else {
            continue;
        };
        let Some(need) = read.needs.get(dependency.need) else {
            continue;
        };
        let settings = dependency
            .manifest
            .map(|id| una_project::ita::parse_ita_manifest(package.sources.file(id).text()).0);
        let version = settings
            .as_ref()
            .and_then(|s| s.version.as_ref())
            .map(|v| v.value.clone())
            .unwrap_or_default();
        let edition = settings
            .as_ref()
            .map_or(una_project::Edition::V0_4, |s| s.edition)
            .name();
        // The package's own profile, `app` when its `ita.una` names none.
        let profile = settings
            .as_ref()
            .and_then(|s| s.ita.as_deref())
            .and_then(|read| read.profile.as_ref())
            .map_or_else(|| "app".to_owned(), |profile| profile.value.clone());
        let ceiling: Vec<String> = need
            .may
            .iter()
            .map(|cap| {
                let ops: Vec<&str> = cap.ops.iter().map(|op| op.value.as_str()).collect();
                format!("{}[{}]", cap.capability, ops.join(", "))
            })
            .collect();
        // One level of needs: the root can grant anything, so a direct dependency's effective
        // ceiling is the ceiling its package extent has (rule 51): the one the root writes, or,
        // for a package with trusted modules, its intersection with the trust line's (spec/30
        // §13 rule 4).
        let extent = manifest
            .boundaries
            .iter()
            .find(|b| b.package && b.module.value == mount.local);
        let granted: una_project::authority::Authority = match extent {
            Some(extent) => extent
                .ceiling
                .iter()
                .flat_map(|cap| {
                    cap.ops
                        .iter()
                        .map(|op| (cap.capability.clone(), op.clone()))
                })
                .collect(),
            None => need
                .may
                .iter()
                .flat_map(|cap| {
                    cap.ops
                        .iter()
                        .map(|op| (cap.capability.clone(), op.value.clone()))
                })
                .collect(),
        };
        let edges = [una_project::authority::Edge {
            from: 0,
            to: 1,
            ceiling: granted.clone(),
        }];
        let effective = una_project::authority::effective(2, 0, &granted, &edges);
        let effective_ceiling = render_authority(effective.get(1).cloned().unwrap_or_default());
        // What its code uses: the `uses` of its functions and handlers.
        let prefix = format!("{}.", mount.local);
        let used = authority_under(analysis, &prefix);
        // The lock's line of the package: by its index for a registry release, by its directory
        // for a path package.
        let locked = lock.as_ref().and_then(|lock| match &dependency.registry {
            Some(origin) => lock.read.packages.get(origin.lock),
            None => lock.read.packages.iter().find(|p| {
                p.name.value == need.name.value
                    && matches!(
                        p.from.as_ref().map(|f| &f.value),
                        Some(una_syntax::ita::LockOrigin::Path(dir)) if dir.trim_end_matches('/') == dependency.dir
                    )
            }),
        });
        // The digest of the code analysed: the bytes loaded, which the checks read.
        let digest = crate::package::dependency_digest(dependency).ok();
        let (source, path, signed) = match &dependency.registry {
            Some(origin) => (
                origin.source.clone(),
                None,
                locked
                    .and_then(|p| p.signed.as_ref())
                    .map(|s| s.value.clone()),
            ),
            None => ("path".to_owned(), Some(dependency.dir.clone()), None),
        };
        out.push(DependencyDoc {
            package: need.name.value.to_string(),
            version,
            local: mount.local.clone(),
            source,
            path,
            digest,
            signed,
            replaced: false,
            edition,
            profile,
            granted_by: vec![GrantedByDoc {
                package: root.clone(),
                source: None,
                may: ceiling,
            }],
            effective_ceiling,
            authority: render_authority(used),
            trust: DependencyTrustDoc {
                modules: manifest
                    .dependency_trusted
                    .iter()
                    .filter(|path| path.starts_with(&prefix))
                    .cloned()
                    .collect(),
            },
        });
    }
    out.sort_by(|a, b| (&a.package, &a.version).cmp(&(&b.package, &b.version)));
    out
}

/// What the code of the modules whose path starts with `prefix` uses: the union of the `uses` of
/// their functions and handlers, as the report's `authority` of a dependency gives it (ADR 0122
/// rule 58), and as a release statement's `authority` must cover it (rule 37). An empty prefix
/// takes every module. O(H) for H headers and handlers.
pub(crate) fn authority_under(
    analysis: &crate::Analysis,
    prefix: &str,
) -> una_project::authority::Authority {
    let mut used = una_project::authority::Authority::new();
    for header in analysis
        .headers
        .iter()
        .filter(|h| h.module.starts_with(prefix))
    {
        for u in &header.uses {
            for op in &u.ops {
                used.insert((u.capability.clone(), op.clone()));
            }
        }
    }
    for module in analysis
        .program
        .modules
        .iter()
        .filter(|m| m.path.starts_with(prefix))
    {
        for decl in &module.decls {
            if let Decl::Handler { uses, .. } = decl {
                for u in uses {
                    for op in &u.ops {
                        used.insert((u.capability.clone(), op.clone()));
                    }
                }
            }
        }
    }
    used
}

/// The registry sources of the program's mounted releases, each with the digest of its root
/// metadata that the lock records (ADR 0122 rule 58), in source order.
fn registries_of(package: &Package, manifest: &una_project::Manifest) -> Vec<RegistryDoc> {
    let Some(id) = package.lock else {
        return Vec::new();
    };
    let lock = una_project::ita::parse_ita_lock(package.sources.file(id).text()).0;
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for dependency in &package.dependencies {
        let Some(origin) = &dependency.registry else {
            continue;
        };
        let mounted = manifest
            .mounts
            .iter()
            .any(|mount| mount.src == format!("{}/src/", dependency.dir));
        if !mounted {
            continue;
        }
        if let Some(registry) = lock
            .read
            .registries
            .iter()
            .find(|r| r.source.value == origin.source)
        {
            out.insert(origin.source.clone(), registry.root.value.clone());
        }
    }
    out.into_iter()
        .map(|(source, root)| RegistryDoc { source, root })
        .collect()
}

/// A set of operations as `Cap[op, op]` entries, by capability.
fn render_authority(set: una_project::authority::Authority) -> Vec<String> {
    let mut by_capability: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (capability, op) in set {
        by_capability.entry(capability).or_default().push(op);
    }
    by_capability
        .into_iter()
        .map(|(capability, ops)| format!("{capability}[{}]", ops.join(", ")))
        .collect()
}

/// The human line of a dependency (ADR 0122 rule 58): `http 1.4.2 (itaty): may Http[get] (by
/// store); uses Http[get]; trust none`.
pub(crate) fn dependency_line(dependency: &DependencyDoc) -> String {
    let origin = match (&dependency.path, &dependency.signed) {
        (Some(path), _) => format!("path {path}"),
        (None, Some(signed)) => format!(
            "{}, signed {}",
            dependency.source,
            signed.split(':').nth(1).unwrap_or_default()
        ),
        (None, None) => dependency.source.clone(),
    };
    let granted: Vec<String> = dependency
        .granted_by
        .iter()
        .map(|g| {
            let may = if g.may.is_empty() {
                "nothing".to_owned()
            } else {
                g.may.join(", ")
            };
            format!("may {may} (by {})", g.package)
        })
        .collect();
    let uses = if dependency.authority.is_empty() {
        "nothing".to_owned()
    } else {
        dependency.authority.join(", ")
    };
    let trust = if dependency.trust.modules.is_empty() {
        "none".to_owned()
    } else {
        dependency.trust.modules.join(", ")
    };
    format!(
        "{} {} ({origin}): {}; uses {uses}; trust {trust}",
        dependency.package,
        dependency.version,
        granted.join("; ")
    )
}
