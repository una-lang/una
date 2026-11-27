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
use una_verify::SourceLayout;

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

/// The entries `out_of_model` gains, at its end, for a package with a `serve:` grant or a
/// `serve` section (ADR 0123 rule 22, and its S4 amendment, section E).
const SERVER_OUT_OF_MODEL: [&str; 3] = [
    "what a server's peers send in reaction to its timing, and which requests its full queues refuse",
    "what a peer receives when the host's deadlines cut an exchange",
    "which request a call of `next` receives when several serve threads hold requests, which the timing of every task of the run and of the serve threads decides",
];

/// The server code of the host, an entry of `tcb` for a package that serves (ADR 0123 rule 22).
const SERVER_CODE: &str = "una-http/serve";

/// The capabilities of the prelude, whose implementations the host provides (`spec/14 §3`), and
/// `HttpServer` of `std.host` (ADR 0123 rule 22).
const HOST_CAPABILITIES: [&str; 7] = [
    "Console",
    "Clock",
    "Env",
    "Files",
    "Keys",
    "Http",
    "HttpServer",
];

/// The premises on the host that acceptance rests on (ADR 0094 rule 5), in the order of
/// `spec/13 §6`: an id and its statement, as `spec/13 §6` writes it. They are assumed of
/// the deployment, never proven: the report lists each with what the package uses it for.
const PREMISES: [(&str, &str); 6] = [
    (
        "H1",
        "whether a read fails depends on the path and on which files exist, not on their content",
    ),
    (
        "H2",
        "whether a write fails, and which files it leaves, does not depend on what it writes",
    ),
    (
        "H3",
        "a response does not depend on the program's type annotations (the host is given text)",
    ),
    (
        "H4",
        "every successful HTTP response is a `Response` of `spec/24`",
    ),
    (
        "Env",
        "the environment variables that exist are the same in both runs; their values may differ",
    ),
    (
        "H5",
        "what an operation answers through a binding where it is labeled l′, and whether it fails, is a function only of the arguments and of the part of the host state at or below l′",
    ),
];

/// Whether the premise `id` covers the host operation `capability.op`: H1 the reads of
/// `Files`, H2 its writes, H3 and H4 every operation of `Http`, and the `Env` note every
/// operation of `Env`. H5 covers labeled operations instead.
fn covers(id: &str, capability: &str, op: &str) -> bool {
    match id {
        "H1" => capability == "Files" && op == "read",
        "H2" => capability == "Files" && op == "write",
        "H3" | "H4" => capability == "Http",
        "Env" => capability == "Env",
        _ => false,
    }
}

/// `una.report/2`, with keys in the order of `spec/13 §6`.
#[derive(Serialize)]
struct Report {
    schema: &'static str,
    package: PackageDoc,
    /// The package's edition and whether it is a preview (ADR 0121 rule 8): additive under
    /// ADR 0076.
    edition: EditionDoc,
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
    /// The host premises, each with what it is used for (ADR 0094 rule 5): additive under
    /// ADR 0076.
    premises: Vec<PremiseDoc>,
    tcb: TcbDoc,
    requirements: Vec<RequirementDoc>,
    coverage: CoverageDoc,
    native: NativeDoc,
    generic_results: Vec<GenericResultDoc>,
    /// The channels outside the model, with the server's at the end for a package that serves
    /// (ADR 0123 rule 22).
    out_of_model: Vec<&'static str>,
    /// What the report warns of, in the report only (ADR 0123 rule 22): `[]` when nothing.
    warnings: Vec<String>,
}

/// A `serve:` grant of the manifest, as the report lists it (ADR 0123 rule 22).
#[derive(Serialize)]
struct ServeGrantDoc {
    /// The address and port, as `127.0.0.1:8080` or `[::1]:8080`.
    address: String,
    /// `http` or `https`.
    scheme: &'static str,
    /// Every limit of `spec/31 §2`, its default applied.
    limits: ServeLimitsDoc,
    /// Whether the host checks `Host` against a list of names: `false` for `hosts=*`.
    checks_host: bool,
}

/// The limits of a `serve:` grant (`spec/31 §2` rule 3), in the order of its table, with sizes
/// in bytes and times in seconds; `null` for `none`.
#[derive(Serialize)]
struct ServeLimitsDoc {
    once: u64,
    rate: Option<String>,
    waiting: u64,
    connections: u64,
    peer: Option<u64>,
    handshakes: u64,
    requests: u64,
    line: u64,
    headers: u64,
    header: u64,
    body: u64,
    buffers: u64,
    peer_buffers: Option<u64>,
    replies: u64,
    peer_replies: Option<u64>,
    delivered: u64,
    idle: u64,
    read: u64,
    write: u64,
    answer: u64,
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
    /// The linker of `una build` (ADR 0124 rule 28): `una <version>` where the internal linker
    /// links, `cc` where the system's links.
    linker: String,
    /// Whether `una build` makes executables a debugger may attach to without `--debuggable`:
    /// not where the internal linker signs them with the hardened runtime (ADR 0124 rule 27,
    /// Question 7); the system linker's are.
    debuggable: bool,
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

/// The edition section of the report (ADR 0121 rule 8, `spec/13 §6`).
#[derive(Serialize)]
struct EditionDoc {
    /// The package's edition, as `ita.una` or `una.toml` names it.
    name: &'static str,
    /// Whether this release serves that edition as a preview (`spec/13 §7`).
    preview: bool,
    /// Each construct of the edition ledger (ADR 0121 rule 9) that the package uses, ordered
    /// by record, then construct; `[]` for a package of an edition with no ledger entry.
    preview_constructs: Vec<PreviewConstructDoc>,
}

/// A construct of the edition ledger that the package uses (ADR 0121 rule 8).
#[derive(Serialize)]
struct PreviewConstructDoc {
    /// The construct's name in the ledger.
    construct: &'static str,
    /// Its record, as `ADR 0116`.
    record: &'static str,
    /// The number of its sites in the package.
    sites: usize,
    /// The names of its unsettled points (ADR 0121 rule 5).
    unsettled: Vec<&'static str>,
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

/// A premise on the host, assumed and not proven, with the host operations
/// (`Capability.op`) or the labeled operations (`module.function: @binding.op gives label`)
/// of the package that rely on it, `[]` when none does.
#[derive(Serialize)]
struct PremiseDoc {
    id: &'static str,
    statement: &'static str,
    used_for: Vec<String>,
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
    /// The `serve:` grants of the manifest, in its order (ADR 0123 rule 22): `[]` for none.
    serve_grants: Vec<ServeGrantDoc>,
    /// The server code of the host, for a package that serves; `null` otherwise.
    server: Option<&'static str>,
    /// The TLS server of `https` grants: `null` while no TLS server is built (#1059).
    tls: Option<&'static str>,
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
    /// The digest of the runtime built into `una`, `native.runtime_sha256` (ADR 0124 rule 28),
    /// from [`crate::runtime_archive::reported_sha256`]; `None` in a development build.
    pub(crate) runtime_sha256: Option<String>,
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
    let document = build(
        &package,
        &analysis,
        counts,
        &trivial,
        key,
        options.runtime_sha256.clone(),
    );
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
    runtime_sha256: Option<String>,
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
    // ADR 0092: the conditions written on the operations of each function's `uses`, from Core
    // IR, by qualified name; shown as `una contract` shows the header's source text.
    let layout = crate::verification::source_layout(package);
    let function_conds: BTreeMap<String, &[una_core::Use]> = program
        .modules
        .iter()
        .flat_map(|module| {
            module.decls.iter().filter_map(|decl| match decl {
                Decl::Fun(fun) => Some((format!("{}.{}", module.path, fun.name), &fun.uses[..])),
                _ => None,
            })
        })
        .collect();
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
    // The host operations the package's functions and handlers use, and its labeled operations.
    let mut host_ops: BTreeSet<(String, String)> = BTreeSet::new();
    let mut labeled: Vec<String> = Vec::new();
    for header in &analysis.headers {
        if header.uses.is_empty() && header.uses_of.is_empty() {
            continue;
        }
        let function = format!("{}.{}", header.module, header.name);
        for used in &header.uses {
            // The host's own capability, not a package capability of the same name.
            if used.host && HOST_CAPABILITIES.contains(&used.capability.as_str()) {
                host.insert(used.capability.clone());
                host_ops.extend(
                    used.ops
                        .iter()
                        .map(|op| (used.capability.clone(), op.clone())),
                );
            }
            // ADR 0094 rule 5: every labeled operation relies on H5, in declaration order.
            labeled.extend(
                used.gives
                    .iter()
                    .map(|(op, label)| format!("{function}: @{}.{op} gives {label}", used.cap)),
            );
        }
        let core = function_conds.get(&function).copied().unwrap_or_default();
        authority.push(AuthorityDoc {
            public: public.get(&function).copied().unwrap_or(false),
            function,
            // ADR 0062: `uses of p` is listed as written, `of p`.
            uses: header
                .uses
                .iter()
                .map(|u| written_use(u, conds_of(core, &u.cap), layout))
                .chain(header.uses_of.iter().map(|param| format!("of {param}")))
                .collect(),
            inferred: header.uses_inferred,
        });
    }
    authority.sort_by(|a, b| a.function.cmp(&b.function));
    // A handler's host operations count as a function's do, and its labeled operations rely on
    // H5 too, after the functions'.
    for handler in &analysis.handler_headers {
        let name = format!("{}.{}", handler.module, handler.name);
        for used in &handler.uses {
            if used.host && HOST_CAPABILITIES.contains(&used.capability.as_str()) {
                host.insert(used.capability.clone());
                host_ops.extend(
                    used.ops
                        .iter()
                        .map(|op| (used.capability.clone(), op.clone())),
                );
            }
            labeled.extend(
                used.gives
                    .iter()
                    .map(|(op, label)| format!("{name}: @{}.{op} gives {label}", used.cap)),
            );
        }
    }
    // ADR 0081 (#14): every handler with `uses`, whose host capabilities count for the TCB too.
    let public_handlers = public_handlers(&modules);
    let handler_headers: BTreeMap<(&str, &str), &una_check::types::HandlerHeader> = analysis
        .handler_headers
        .iter()
        .map(|h| ((h.module.as_str(), h.name.as_str()), h))
        .collect();
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
            // Its host capabilities were counted from `handler_headers`, which tell the host's
            // from a package capability of the same name.
            let handler = format!("{}.{name}", module.path);
            // The labels as written, from the checker's header of the handler (ADR 0094 rule 1).
            let written = handler_headers.get(&(module.path.as_str(), name.as_str()));
            let uses = uses
                .iter()
                .map(|u| {
                    let gives = written
                        .and_then(|h| h.uses.iter().find(|w| w.cap == u.cap))
                        .map_or(&[][..], |w| &w.gives[..]);
                    render_use(&u.cap, &u.capability, &u.ops, gives, &u.conds, layout)
                })
                .collect();
            handler_authority.push(HandlerAuthorityDoc {
                public: public_handlers.contains(&handler),
                handler,
                capability: capability_path.qualified(),
                uses,
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
    let premises = PREMISES
        .iter()
        .map(|(id, statement)| PremiseDoc {
            id,
            statement,
            used_for: if *id == "H5" {
                labeled.clone()
            } else {
                host_ops
                    .iter()
                    .filter(|(capability, op)| covers(id, capability, op))
                    .map(|(capability, op)| format!("{capability}.{op}"))
                    .collect()
            },
        })
        .collect();
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
    // ADR 0123 rule 22: a package serves when it has a `serve:` grant or a `serve` section.
    let (grant_docs, warnings) = serve_grants(&manifest);
    let serves = !grant_docs.is_empty() || manifest.serve.is_some();
    if serves {
        host.insert("HttpServer".to_owned());
    }
    let mut out_of_model = OUT_OF_MODEL.to_vec();
    if serves {
        out_of_model.extend(SERVER_OUT_OF_MODEL);
    }
    Report {
        schema: REPORT_SCHEMA,
        package: package_doc,
        edition: EditionDoc {
            name: manifest.edition.name(),
            preview: manifest.edition.is_preview(),
            preview_constructs: crate::preview::preview_constructs(package, analysis, &manifest)
                .into_iter()
                .map(|(entry, sites)| PreviewConstructDoc {
                    construct: entry.name,
                    record: entry.record,
                    sites,
                    unsettled: entry.unsettled.iter().map(|point| point.name).collect(),
                })
                .collect(),
        },
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
        premises,
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
            serve_grants: grant_docs,
            server: serves.then_some(SERVER_CODE),
            tls: None,
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
            runtime_sha256,
            linker: crate::build::linker_name(),
            debuggable: crate::build::linker_name() == "cc",
        },
        generic_results,
        out_of_model,
        warnings,
    }
}

/// The conditions that the Core `uses` entries `core` write on binding `cap`'s operations
/// (ADR 0092), empty when none is written.
fn conds_of<'a>(core: &'a [una_core::Use], cap: &str) -> &'a [(String, una_core::Clause)] {
    core.iter()
        .find(|u| u.cap == cap)
        .map_or(&[][..], |u| &u.conds[..])
}

/// A function's `uses` entry as its header writes it, for `authority` (`spec/13 §6`).
fn written_use(
    u: &una_check::types::HeaderUse,
    conds: &[(String, una_core::Clause)],
    layout: SourceLayout,
) -> String {
    render_use(&u.cap, &u.capability, &u.ops, &u.gives, conds, layout)
}

/// `@cap: Capability[op, op gives l where C]`: a `uses` entry as a header writes it, each
/// operation with its label (ADR 0094 rule 1) and its condition (ADR 0092), in the canonical
/// form `una contract` shows (`spec/13 §6`). An operation with neither prints as its name, so
/// the entries of edition `0.1` and `0.2` packages are unchanged. O(ops * (labels + conds)).
fn render_use(
    cap: &str,
    capability: &str,
    ops: &[String],
    gives: &[(String, String)],
    conds: &[(String, una_core::Clause)],
    layout: SourceLayout,
) -> String {
    let ops: Vec<String> = ops
        .iter()
        .map(|op| {
            let mut text = op.clone();
            if let Some((_, label)) = gives.iter().find(|(o, _)| o == op) {
                text.push_str(" gives ");
                text.push_str(label);
            }
            if let Some((_, cond)) = conds.iter().find(|(o, _)| o == op) {
                text.push_str(" where ");
                match layout {
                    SourceLayout::Joined => text.push_str(&cond.text),
                    SourceLayout::Wrapped => text.push_str(&una_verify::unwrap_layout(&cond.text)),
                }
            }
            text
        })
        .collect();
    format!("@{cap}: {capability}[{}]", ops.join(", "))
}

/// The `serve:` grants of the manifest as the report lists them, and its warnings about them
/// (ADR 0123 rule 22): a plain `http` grant on an address that is not loopback, a wildcard
/// included; `hosts=*`; `peer=none`; `peer_buffers=none`; `tls12`. O(grants).
fn serve_grants(manifest: &una_project::Manifest) -> (Vec<ServeGrantDoc>, Vec<String>) {
    let mut grants = Vec::new();
    let mut warnings = Vec::new();
    for spec in &manifest.grants {
        if !spec.value.starts_with(una_core::http::serve::PREFIX) {
            continue;
        }
        // A grant that does not parse was refused by the manifest's checks (U7008).
        let Ok(grant) = una_core::http::serve::ServeGrant::parse(&spec.value) else {
            continue;
        };
        let address = grant.listener();
        let limits = &grant.limits;
        if !grant.tls && !grant.address.is_loopback() {
            warnings.push(format!(
                "the grant `{address}` serves plain `http` on an address that is not loopback, so its requests and replies cross the network unencrypted"
            ));
        }
        if grant.hosts == una_core::http::serve::Hosts::Any {
            warnings.push(format!(
                "the grant `{address}` has `hosts=*`, so the host accepts every name in `Host`"
            ));
        }
        if limits.peer.is_none() {
            warnings.push(format!(
                "the grant `{address}` has `peer=none`, so one peer may hold every connection"
            ));
        }
        if limits.peer_buffers.is_none() {
            warnings.push(format!(
                "the grant `{address}` has `peer_buffers=none`, so one peer may hold every request buffer"
            ));
        }
        if grant.tls12 {
            warnings.push(format!(
                "the grant `{address}` has `tls12`, so it accepts TLS 1.2 beside TLS 1.3"
            ));
        }
        grants.push(ServeGrantDoc {
            address,
            scheme: if grant.tls { "https" } else { "http" },
            limits: ServeLimitsDoc {
                once: limits.once,
                rate: limits
                    .rate
                    .map(|rate| format!("{}/{}", rate.count, rate.period.name())),
                waiting: limits.waiting,
                connections: limits.connections,
                peer: limits.peer,
                handshakes: limits.handshakes,
                requests: limits.requests,
                line: limits.line,
                headers: limits.headers,
                header: limits.header,
                body: limits.body,
                buffers: limits.buffers,
                peer_buffers: limits.peer_buffers,
                replies: limits.replies,
                peer_replies: limits.peer_replies,
                delivered: limits.delivered,
                idle: limits.idle,
                read: limits.read,
                write: limits.write,
                answer: limits.answer,
            },
            checks_host: grant.hosts != una_core::http::serve::Hosts::Any,
        });
    }
    (grants, warnings)
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
            Expr::Start { task, .. } | Expr::Quiet { body: task, .. } => self.expr(task),
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
/// The constructs of the human `edition:` line (ADR 0121 rule 8): `: for ever (ADR 0116, 3
/// sites), sum (ADR 0120, 1 site; unsettled: ...)`, or nothing when the package uses none.
fn preview_constructs_line(constructs: &[PreviewConstructDoc]) -> String {
    if constructs.is_empty() {
        return String::new();
    }
    let items: Vec<String> = constructs
        .iter()
        .map(|c| {
            let sites = if c.sites == 1 { "site" } else { "sites" };
            let unsettled = if c.unsettled.is_empty() {
                String::new()
            } else {
                format!("; unsettled: {}", c.unsettled.join(", "))
            };
            format!(
                "{} ({}, {} {sites}{unsettled})",
                c.construct, c.record, c.sites
            )
        })
        .collect();
    format!(": {}", items.join(", "))
}

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
    let preview = if report.edition.preview {
        " (preview)"
    } else {
        ""
    };
    line(format!(
        "edition: {}{preview}{}",
        report.edition.name,
        preview_constructs_line(&report.edition.preview_constructs)
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
    // ADR 0094 rule 5: what the package assumes of the host, never proven.
    line("assumed of the host:".to_owned());
    for premise in &report.premises {
        let used_for = if premise.used_for.is_empty() {
            "nothing".to_owned()
        } else {
            premise.used_for.join(", ")
        };
        line(format!("  {}: used for {used_for}", premise.id));
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
    if let Some(server) = report.tcb.server {
        line(format!("  server: {server}"));
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
    line(format!("  linker: {}", report.native.linker));
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
    if !report.tcb.serve_grants.is_empty() {
        line("servers:".to_owned());
        for grant in &report.tcb.serve_grants {
            let limits = &grant.limits;
            let none =
                |value: Option<u64>| value.map_or_else(|| "none".to_owned(), |v| v.to_string());
            line(format!(
                "  {}://{}: {}/once, rate {}, waiting {}, connections {}, peer {}, body {} bytes, buffers {} bytes, peer buffers {}, answer within {} s, {}",
                grant.scheme,
                grant.address,
                limits.once,
                limits.rate.as_deref().unwrap_or("none"),
                limits.waiting,
                limits.connections,
                none(limits.peer),
                limits.body,
                limits.buffers,
                none(limits.peer_buffers),
                limits.answer,
                if grant.checks_host {
                    "checks Host"
                } else {
                    "accepts every Host"
                }
            ));
        }
    }
    line(format!("out of model: {}", report.out_of_model.join(", ")));
    for warning in &report.warnings {
        line(format!("warning: {warning}"));
    }
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
        .map(|id| una_project::ita::parse_ita_lock(package.sources.file(id)).0);
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
            .map(|id| una_project::ita::parse_ita_manifest(package.sources.file(id)).0);
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
    let lock = una_project::ita::parse_ita_lock(package.sources.file(id)).0;
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
