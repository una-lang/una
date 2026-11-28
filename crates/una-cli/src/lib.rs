//! Command implementations of the `una` binary (`spec/13 §5`), the only crate that does I/O to
//! the terminal.
//!
//! The binary hands its arguments, standard streams and environment variables to [`run`];
//! nothing here reads the process environment, so every command also runs in-process.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod args;
mod build;
mod contract;
mod core_json;
mod fix;
mod ita;
mod ita_registry;
mod lsp;
mod migrate;
mod musl_inputs;
mod package;
mod preview;
mod proof_key;
mod recovery;
mod registry;
mod report;
mod runtime;
mod runtime_archive;
mod std_builtins;
mod std_listing;
mod trace;
mod verification;

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Parser;
use una_check::{
    HandlerHeader, Header, ModuleInput, PackageInput, PendingObligation, Rejected, RunInput,
    check_types, check_types_with_rejected,
};
use una_diag::{Code, Diagnostic, render_human, render_json};
use una_syntax::ast::File;
use una_syntax::{FileId, Interner, KeywordSet, Lexed, SourceDb, Span, lex_in, parse};

use crate::args::{Cli, ColorChoice, Command, ItaCommand, TierArg};
pub use crate::package::{LoadError, Package, load};
pub use crate::std_builtins::{
    Builtin, STD_BUILTINS_SCHEMA, builtin_docs, builtins, std_builtin_items, std_builtins,
};
pub use crate::std_listing::{Item, STD_LISTING_SCHEMA, Variant, std_items, std_listing};
pub use crate::verification::{DEFAULT_WALLCLOCK, Solving, Verification};
pub mod solver_child;

/// The exit status of a `una` command (`spec/13 §5`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitStatus {
    /// `0`: success.
    Success,
    /// `1`: errors in the program or failed tests.
    Errors,
    /// `2`: usage or configuration error.
    Usage,
    /// `3`: internal compiler error.
    Internal,
}

impl ExitStatus {
    /// The process exit code.
    pub fn code(self) -> u8 {
        match self {
            ExitStatus::Success => 0,
            ExitStatus::Errors => 1,
            ExitStatus::Usage => 2,
            ExitStatus::Internal => 3,
        }
    }
}

/// The tiers of `una check` (`spec/12 §1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Every check except proof obligations.
    Zero,
    /// Tier 0 and every proof obligation.
    One,
}

/// The outcome of [`check`]: its diagnostics, the sources they point into, the Core IR of
/// the declarations without diagnostics, and at tier 1 the obligations.
#[derive(Debug)]
pub struct Checked {
    /// The diagnostics, in reporting order.
    pub diagnostics: Vec<Diagnostic>,
    /// The loaded sources.
    pub sources: SourceDb,
    /// The Core IR (`spec/09`).
    pub program: una_core::Program,
    /// The obligations and their outcomes, when tier 1 ran.
    pub verification: Option<Verification>,
    /// The obligation sites that the type checker found, which the verifier generates over
    /// Core IR (`spec/12 §3`).
    pub pending: Vec<PendingObligation>,
    /// The graph of ADR 0116 P1 and its construct trees as the type checker recorded them,
    /// for the completeness test of P1 (`tests/timing_graph.rs`).
    pub timing_graph: una_check::TimingGraph,
}

/// Loads and checks the file or package at `path` (`spec/13 §5`, `spec/12 §1`), with the
/// default wall-clock guard of the solver and no proof cache.
pub fn check(path: &Path, tier: Tier) -> Result<Checked, LoadError> {
    check_with(path, tier, &Solving::new(DEFAULT_WALLCLOCK))
}

/// Loads and checks the file or package at `path`: tier 0, then, at tier 1 and when tier 0
/// reports nothing, every proof obligation.
pub fn check_with(path: &Path, tier: Tier, solving: &Solving) -> Result<Checked, LoadError> {
    let package = package::load_in(
        path,
        &std::collections::BTreeMap::new(),
        solving.itaoca.as_ref(),
    )?;
    let (mut diagnostics, program, pending, timing_graph) = on_large_stack(|| {
        let analysis = analyze(&package, None);
        (
            analysis.diagnostics,
            analysis.program,
            analysis.pending,
            analysis.timing_graph,
        )
    });
    let mut verification = None;
    if tier == Tier::One && diagnostics.is_empty() {
        let found = verification::tier_one(&package, &program, solving);
        diagnostics.extend(found.diagnostics);
        verification = found.verification;
    }
    Ok(Checked {
        diagnostics,
        sources: package.sources,
        program,
        verification,
        pending,
        timing_graph,
    })
}

/// What checking a package finds: its diagnostics, the tree of each module, the names of the
/// trees, the headers of its functions after inference, its Core IR, and the module that
/// `una run` runs.
struct Analysis {
    diagnostics: Vec<Diagnostic>,
    trees: Vec<(FileId, File)>,
    interner: Interner,
    headers: Vec<Header>,
    /// The `uses` of the package's handlers, with their operations' labels (ADR 0094).
    handler_headers: Vec<HandlerHeader>,
    program: una_core::Program,
    entry: Option<String>,
    pending: Vec<PendingObligation>,
    /// Each name that resolves to a declaration or a local, and its declaration (ADR 0063).
    references: Vec<(una_syntax::Span, una_syntax::Span)>,
    /// What the type checker resolved, by span (ADR 0063).
    index: una_check::Index,
    /// The lambdas given to `of` parameters, by span, with their effects and failure (#443).
    of_lambdas: Vec<(una_syntax::Span, bool, bool)>,
    /// The package's file-state label (ADR 0085), which `una core` exports (#467).
    file_state: una_core::Label,
    /// The mounted dependencies of the program (ADR 0122 rule 47).
    mounts: Vec<una_project::Mount>,
    /// The sites with deadlines and their timing labels (ADR 0116 rule 21), for the report.
    timing: Vec<una_check::TimingSite>,
    /// `L_lim` (ADR 0117 IF4), which a grant with limits needs at most `internal`.
    limit_label: una_core::Label,
    /// The units of the state types of the `[drive]` module, which Core IR erases
    /// (`spec/26 §2` rule 2), for the shape check of `una run` and `una build`.
    drive_units: Option<una_check::DriveUnits>,
    /// The graph of ADR 0116 P1 as the type checker recorded it, for [`Checked`].
    timing_graph: una_check::TimingGraph,
    /// The constructs of the edition ledger that only the checker sees, by span, for the
    /// report's `preview_constructs` (ADR 0121 rule 8).
    preview_sites: Vec<(una_project::ledger::Construct, una_syntax::Span)>,
}

/// Checks `package`; with `run_grants`, for `una run`, whose grants are final (U3012).
fn analyze(package: &Package, run_grants: Option<&[String]>) -> Analysis {
    analyze_and(package, run_grants, false).0
}

/// [`analyze`], and with `with_rejected` the Core IR of the rejected declarations whose
/// elaboration is complete (ADR 0091), for `una core --with-rejected` only. The [`Analysis`]
/// is the one [`analyze`] returns.
fn analyze_and(
    package: &Package,
    run_grants: Option<&[String]>,
    with_rejected: bool,
) -> (Analysis, Vec<Rejected>) {
    let (manifest, mut diagnostics) = package::project(package);
    // ADR 0114 rule 6.4: a package of edition 0.3 may hold floats, whose literals the checker
    // converts, so the environment is tested before any source is read; U6006 stops here.
    if manifest.edition >= una_project::Edition::V0_3
        && let Err(probe) = una_core::float::probe_environment()
    {
        return (environment_failure(probe), Vec::new());
    }
    // ADR 0120 rule 5: edition 0.5 adds the binary32 probes, for its `F32` literals.
    if manifest.edition.has_collections_and_numerics()
        && let Err(probe) = una_core::float32::probe_environment()
    {
        return (environment_failure(probe), Vec::new());
    }
    let mut interner = Interner::new();
    let mut trees = Vec::new();
    // ADR 0096 C: the poisoned names of each module with broken declarations, and those
    // declarations.
    let mut poison = std::collections::BTreeMap::new();
    let mut broken_files = std::collections::BTreeMap::new();
    // The program's modules: the root's, then those of each mounted dependency (ADR 0122
    // rule 48). A dependency that is not mounted is never checked as part of the program.
    // Each module is read under its own package's edition (rules 48 and 49): a dependency of
    // an earlier edition keeps its keywords, as `let` in edition 0.4 under a root of 0.5.
    let editions: Vec<(FileId, una_project::Edition)> = package
        .modules
        .iter()
        .map(|&id| (id, manifest.edition))
        .chain(package.dependencies.iter().flat_map(|dependency| {
            let src = format!("{}/src/", dependency.dir);
            manifest
                .mounts
                .iter()
                .find(|mount| mount.src == src)
                .into_iter()
                .flat_map(|mount| dependency.modules.iter().map(|&id| (id, mount.edition)))
        }))
        .collect();
    let keyed: Vec<(FileId, KeywordSet)> = editions
        .iter()
        .map(|&(id, edition)| (id, keyword_set(edition)))
        .collect();
    for checked in check_files(&package.sources, &keyed, &mut interner) {
        // A module is broken as a whole only when its header is (ADR 0096 rule 7).
        let whole = checked.broken.as_ref().is_some_and(|broken| broken.whole);
        diagnostics.extend(checked.syntax);
        if let Some(canonical) = checked.canonical {
            diagnostics.extend(una_fmt::differences(
                &package.sources,
                checked.id,
                &canonical,
            ));
        }
        diagnostics.extend(checked.partial_canonical);
        if let Some(broken) = checked.broken {
            if !broken.whole {
                poison.insert(checked.id, broken.poison());
            }
            broken_files.insert(checked.id, broken);
        }
        trees.push((checked.id, checked.tree, whole));
    }
    // The entry is `[run] entry`, or the module of a single file.
    let entry = if package.single {
        trees.first().and_then(|(_, tree, _)| {
            let header = tree.module.as_ref()?;
            let path: Vec<&str> = header
                .path
                .segments
                .iter()
                .map(|segment| interner.resolve(segment.name))
                .collect();
            Some(path.join("."))
        })
    } else {
        manifest.entry.as_ref().map(|entry| entry.value.clone())
    };
    // The final grants of `una run` check the functions the host calls: `main` of the entry,
    // or `handle` and `on_stop` of a `serve` module (`spec/31 §7` rule 3), which has no entry.
    let served = manifest
        .serve
        .as_ref()
        .map(|serve| serve.module.value.as_str());
    let run = match (run_grants, entry.as_deref().or(served)) {
        (Some(grants), Some(entry)) => Some(RunInput { entry, grants }),
        _ => None,
    };
    let locks = locks(package, &manifest, &mut interner);
    let input = PackageInput {
        sources: &package.sources,
        interner: &interner,
        single: package.single,
        modules: trees
            .iter()
            .map(|(id, tree, broken)| ModuleInput {
                id: *id,
                tree,
                broken: *broken,
            })
            .collect(),
        manifest: package.manifest,
        settings: &manifest,
        run,
        earlier_errors: false,
        locks: &locks,
    };
    let (names, references) = if poison.is_empty() {
        una_check::check_names_indexed(&input)
    } else {
        una_check::check_names_poisoned(&input, &poison)
    };
    diagnostics.extend(names);
    // The syntax, manifest and name errors so far, which the type check does not see.
    let input = PackageInput {
        earlier_errors: diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == una_diag::Severity::Error),
        ..input
    };
    let (types, rejected) = if with_rejected {
        check_types_with_rejected(&input)
    } else {
        (check_types(&input), Vec::new())
    };
    diagnostics.extend(types.diagnostics);
    if !broken_files.is_empty() {
        withhold_dependents(
            &package.sources,
            &keyed,
            &manifest,
            &broken_files,
            &mut diagnostics,
        );
    }
    // ADR 0137 rule 10: the messages of a package of edition 0.5 name its binding words; a
    // text about a module of a dependency of an earlier edition keeps that edition's words,
    // decided for the main location and for each related note on its own.
    if manifest.edition >= una_project::Edition::V0_5 {
        let earlier: std::collections::BTreeSet<&str> = editions
            .iter()
            .filter(|(_, edition)| *edition < una_project::Edition::V0_5)
            .map(|(id, _)| package.sources.file(*id).path())
            .collect();
        for diagnostic in &mut diagnostics {
            una_diag::binding_words::in_edition_0_5_where(diagnostic, |file| {
                !earlier.contains(file)
            });
        }
    }
    let analysis = Analysis {
        mounts: manifest.mounts.clone(),
        diagnostics,
        trees: trees.into_iter().map(|(id, tree, _)| (id, tree)).collect(),
        interner,
        headers: types.headers,
        handler_headers: types.handler_headers,
        program: types.program,
        entry,
        pending: types.obligations,
        references,
        index: types.index,
        of_lambdas: types.of_lambdas,
        file_state: types.file_state,
        timing: types.timing,
        preview_sites: types.preview_sites,
        limit_label: types.limit_label,
        drive_units: types.drive_units,
        timing_graph: types.timing_graph,
    };
    (analysis, rejected)
}

/// The analysis of a package of edition 0.3 whose floating-point environment failed the probe
/// `probe` (ADR 0114 rule 6.4): U6006 alone, at the start of `una.toml`, and nothing checked.
fn environment_failure(probe: &str) -> Analysis {
    let start = una_diag::Position { line: 1, col: 1 };
    let location = una_diag::Location {
        file: "una.toml".to_owned(),
        start,
        end: start,
    };
    Analysis {
        diagnostics: vec![Diagnostic::error(
            Code::U6006,
            format!(
                "the floating-point environment of this platform is not the default one ({probe})"
            ),
            location,
        )],
        trees: Vec::new(),
        interner: Interner::new(),
        headers: Vec::new(),
        handler_headers: Vec::new(),
        program: una_core::Program::default(),
        entry: None,
        pending: Vec::new(),
        references: Vec::new(),
        index: una_check::Index::default(),
        of_lambdas: Vec::new(),
        file_state: una_core::Label::public(),
        mounts: Vec::new(),
        timing: Vec::new(),
        preview_sites: Vec::new(),
        limit_label: una_core::Label::public(),
        drive_units: None,
        timing_graph: una_check::TimingGraph::default(),
    }
}

/// The locked functions of `[contracts]`, each clause parsed where its string is written
/// (ADR 0092 rule 8, as amended). A string with escapes, or a multi-line one, is parsed from
/// its value, and each span is placed where its bytes are written (#727). A string that cannot
/// be read back to its value is a syntax error of the clause, never a lock left out.
fn locks(
    package: &Package,
    manifest: &una_project::Manifest,
    interner: &mut Interner,
) -> Vec<una_check::Lock> {
    let Some(file) = package.manifest else {
        return Vec::new();
    };
    let written = package.sources.file(file).text();
    manifest
        .contracts
        .iter()
        .map(|contract| una_check::Lock {
            function: contract.function.value.clone(),
            key: Span::new(
                file,
                contract.function.range.start,
                contract.function.range.end,
            ),
            clauses: contract
                .clauses
                .iter()
                .map(|clause| {
                    let span = Span::new(file, clause.range.start, clause.range.end);
                    let offsets = if manifest.format == una_project::Format::Ita {
                        // In `ita.una` a clause is its own line: no quotes and no escapes.
                        Some(una_project::literal::Offsets::identity(clause.value.len()))
                    } else {
                        written.get(clause.range.clone()).and_then(|literal| {
                            una_project::literal::offsets(literal, &clause.value)
                        })
                    };
                    let parsed = match &offsets {
                        Some(offsets) => {
                            let base = clause.range.start;
                            let place = |start: usize, end: usize| {
                                let (start, end) = offsets.place(start, end);
                                (base + start, base + end)
                            };
                            una_syntax::parse_uses_clause(
                                &clause.value,
                                file,
                                &place,
                                written,
                                interner,
                            )
                            .map_err(|errors| {
                                errors.into_iter().next().unwrap_or_else(|| {
                                    una_syntax::SyntaxError::new(
                                        una_diag::Code::U0101,
                                        span,
                                        "the clause does not parse",
                                    )
                                })
                            })
                        }
                        None => Err(una_syntax::SyntaxError::new(
                            una_diag::Code::U0101,
                            span,
                            "its string could not be read back to the positions it is written at",
                        )),
                    };
                    una_check::LockClause {
                        span,
                        parsed,
                        value: clause.value.clone(),
                        offsets,
                    }
                })
                .collect(),
        })
        .collect()
}

/// ADR 0096 rule 9, as amended: drops every diagnostic but syntax of the declarations that
/// reach a broken one.
fn withhold_dependents(
    sources: &SourceDb,
    keyed: &[(FileId, KeywordSet)],
    manifest: &una_project::Manifest,
    broken_files: &std::collections::BTreeMap<FileId, recovery::BrokenFile>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let none = std::collections::BTreeSet::new();
    let modules: Vec<FileId> = keyed.iter().map(|(id, _)| *id).collect();
    let clean: Vec<(FileId, Vec<una_syntax::Declaration>)> = keyed
        .iter()
        .filter(|(id, _)| !broken_files.contains_key(id))
        .map(|&(id, keywords)| {
            (
                id,
                una_syntax::declarations_in(sources.file(id).text(), keywords),
            )
        })
        .collect();
    // A dependency's module `m` is `<local>.m` in the program, and so are the modules it
    // imports, but the standard ones (ADR 0122 rule 47).
    let mounted: std::collections::BTreeMap<FileId, Vec<una_syntax::Declaration>> = modules
        .iter()
        .filter_map(|&id| {
            let local = &manifest.mount_of(sources.file(id).path())?.local;
            let declarations = match broken_files.get(&id) {
                Some(file) => file.declarations.clone(),
                None => clean.iter().find(|(c, _)| *c == id)?.1.clone(),
            };
            let renamed = declarations
                .into_iter()
                .map(|mut declaration| {
                    let rename = match declaration.what {
                        una_syntax::DeclarationKind::Header => true,
                        una_syntax::DeclarationKind::Import => declaration
                            .module
                            .as_deref()
                            .is_some_and(|module| !module.starts_with("std.")),
                        _ => false,
                    };
                    if rename && let Some(module) = &declaration.module {
                        declaration.module = Some(format!("{local}.{module}"));
                    }
                    declaration
                })
                .collect();
            Some((id, renamed))
        })
        .collect();
    let mut texts = Vec::new();
    for &id in &modules {
        let (declarations, broken) = match broken_files.get(&id) {
            Some(file) => (file.declarations.as_slice(), &file.broken),
            None => match clean.iter().find(|(clean_id, _)| *clean_id == id) {
                Some((_, declarations)) => (declarations.as_slice(), &none),
                None => continue,
            },
        };
        let declarations = mounted.get(&id).map_or(declarations, Vec::as_slice);
        let module = declarations
            .iter()
            .find(|d| d.what == una_syntax::DeclarationKind::Header)
            .and_then(|d| d.module.as_deref())
            .unwrap_or("");
        texts.push(recovery::ModuleText {
            path: sources.file(id).path(),
            module,
            declarations,
            broken,
        });
    }
    recovery::withhold(diagnostics, &texts);
}

/// The syntax diagnostics of one file, its tree, and its canonical form when it has no
/// syntax error. A file with one has its broken declarations, its tree without them, and the
/// canonical-form diagnostics of the declarations that parsed (ADR 0096 C).
struct FileCheck {
    id: FileId,
    syntax: Vec<Diagnostic>,
    tree: File,
    canonical: Option<String>,
    broken: Option<recovery::BrokenFile>,
    partial_canonical: Vec<Diagnostic>,
}

/// A parsed file: its id, tokens, tree, syntax diagnostics, and the byte offsets of its syntax
/// errors.
type Parsed = (FileId, Lexed, File, Vec<Diagnostic>, Vec<usize>);

/// A file with broken declarations, without them: its text, parsed, for the canonical form of
/// the declarations that parsed (ADR 0096 rule 10).
struct Partial {
    sources: SourceDb,
    id: FileId,
    lexed: Lexed,
    tree: File,
    text: String,
    kept: Vec<recovery::Kept>,
}

/// Lexes the files `modules` of a package, chooses their renames together (#125), parses
/// them, then prints each one that parsed in its canonical form, knowing the constructors that
/// every such file declares (ADR 0020 rule 6).
fn check_files(
    sources: &SourceDb,
    modules: &[(FileId, KeywordSet)],
    interner: &mut Interner,
) -> Vec<FileCheck> {
    let mut lexed: Vec<Lexed> = modules
        .iter()
        .map(|&(id, keywords)| lex_in(sources.file(id), id, interner, keywords))
        .collect();
    // A word renamed in one module is renamed alike in every module that names it (#125).
    una_syntax::rename_across(&mut lexed, interner);
    let parsed: Vec<Parsed> = modules
        .iter()
        .zip(lexed)
        .map(|(&(id, _), lexed)| {
            let parsed = parse(&lexed, id, interner);
            // An error the parser reads past keeps its declaration (ADR 0105 rule 3).
            let errors = lexed
                .errors
                .iter()
                .chain(&parsed.errors)
                .filter(|error| !error.recovered)
                .map(|error| error.span.start)
                .collect();
            let syntax: Vec<Diagnostic> = lexed
                .errors
                .iter()
                .chain(&parsed.errors)
                .map(|error| error.to_diagnostic(sources))
                .collect();
            (id, lexed, parsed.file, syntax, errors)
        })
        .collect();
    // ADR 0096 C: the broken declarations of each file with a syntax error, its tree without
    // them, and its text without them, parsed again once.
    let mut files = Vec::new();
    for (id, lexed, mut tree, syntax, errors) in parsed {
        let (broken, partial) = if syntax.is_empty() {
            (None, None)
        } else {
            // The file's own keyword set, as its lexer read it.
            let keywords = lexed.keywords;
            let text = sources.file(id).text();
            let broken = recovery::BrokenFile::new(text, &errors, keywords);
            tree.decls.retain(|decl| !broken.in_broken(decl.span.start));
            tree.imports
                .retain(|import| !broken.in_broken(import.span.start));
            let partial = (!broken.whole)
                .then(|| partial_of(sources.file(id).path(), text, &broken, interner, keywords))
                .flatten();
            (Some(broken), partial)
        };
        files.push((id, lexed, tree, syntax, broken, partial));
    }
    let constructors = una_fmt::Constructors::new(
        files
            .iter()
            .filter(|(_, _, _, syntax, _, _)| syntax.is_empty())
            .map(|(_, _, tree, _, _, _)| tree)
            .chain(
                files
                    .iter()
                    .filter_map(|(_, _, _, _, _, partial)| partial.as_ref().map(|p| &p.tree)),
            ),
    );
    files
        .into_iter()
        .map(|(id, lexed, tree, syntax, broken, partial)| {
            // A file with syntax errors has no canonical form as a whole.
            let canonical = syntax.is_empty().then(|| {
                una_fmt::format_with(sources.file(id), &lexed, &tree, interner, &constructors)
            });
            let partial_canonical = partial.map_or_else(Vec::new, |partial| {
                let canonical = una_fmt::format_with(
                    partial.sources.file(partial.id),
                    &partial.lexed,
                    &partial.tree,
                    interner,
                    &constructors,
                );
                recovery::canonical_by_declaration(
                    sources.file(id).path(),
                    &partial.text,
                    &canonical,
                    &partial.kept,
                )
            });
            FileCheck {
                id,
                syntax,
                tree,
                canonical,
                broken,
                partial_canonical,
            }
        })
        .collect()
}

/// The text of a file without its broken declarations, parsed; `None` when it does not parse,
/// so that no canonical form is compared.
fn partial_of(
    path: &str,
    text: &str,
    broken: &recovery::BrokenFile,
    interner: &mut Interner,
    keywords: KeywordSet,
) -> Option<Partial> {
    let (kept_text, kept) = broken.kept(text);
    let mut partial_sources = SourceDb::new();
    let id = partial_sources.add(path, kept_text.as_bytes());
    let lexed = lex_in(partial_sources.file(id), id, interner, keywords);
    let parsed = parse(&lexed, id, interner);
    (lexed.errors.is_empty() && parsed.errors.is_empty()).then_some(Partial {
        sources: partial_sources,
        id,
        lexed,
        tree: parsed.file,
        text: kept_text,
        kept,
    })
}

/// The stack of the thread that checks a package. The passes recurse over the syntax tree,
/// whose depth the parser bounds (`una_syntax::MAX_NESTING`); unoptimized builds use up to
/// about 40 KiB of stack per level. The passes over Core IR, the verifier's included, recurse
/// once per nested Core expression, which the checker bounds (`una_core::depth::MAX_DEPTH`,
/// #2264). The stack is reserved, not committed.
const CHECK_STACK_BYTES: usize = una_core::depth::STACK_BYTES;

/// Runs `work` on a thread with [`CHECK_STACK_BYTES`] of stack, or on the current thread when
/// the operating system cannot start one.
fn on_large_stack<T: Send>(work: impl Fn() -> T + Sync) -> T {
    std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new()
            .name("una-check".to_owned())
            .stack_size(CHECK_STACK_BYTES)
            .spawn_scoped(scope, &work);
        match spawned {
            Ok(handle) => handle
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload)),
            Err(_) => work(),
        }
    })
}

/// The environment variables of the process, by name, which `una run` gives to programs that
/// use `Env.get` (`spec/14 §3`). The binary passes the real ones; the library never reads them
/// (`compiler/conventions.md §1`). A value that is not valid UTF-8 is [`una_interp::NotText`]:
/// `Env.get` aborts on it (S-16, ADR 0086), and the toolchain's own lookups treat it as unset.
pub type Environment<'a> = &'a (dyn Fn(&str) -> Result<Option<String>, una_interp::NotText> + Sync);

/// The variable `name` for the toolchain's own use: unset when its value is not valid UTF-8.
pub(crate) fn variable(environment: Environment<'_>, name: &str) -> Option<String> {
    environment(name).ok().flatten()
}

/// Itaoca, the store of fetched packages (ADR 0122 rule 30): the directory `UNA_ITAOCA` names
/// when it is set and not empty, else `.una/itaoca` in `HOME`.
pub(crate) fn itaoca(environment: Environment<'_>) -> Option<una_project::itaoca::Itaoca> {
    let dir = variable(environment, "UNA_ITAOCA");
    let home = variable(environment, "HOME");
    una_project::itaoca::Itaoca::at(dir.as_deref(), home.as_deref().map(Path::new))
}

/// The codes `una ita audit` reports for the package at `path`, each registry package's facts
/// from `facts_of`, as the verified client gives them (ADR 0122 rule 41, U7014): for the explain
/// examples of U7014, which state a registry's state without a registry.
pub fn audit_codes(
    path: &Path,
    facts_of: &mut dyn FnMut(
        &una_project::registry::audit::Locked,
    ) -> Result<una_project::registry::audit::Facts, String>,
) -> Vec<Code> {
    ita_registry::audit_codes(path, facts_of)
}

/// The local clock, in seconds since the Unix epoch, which only the freshness of a registry's
/// checkpoint reads (ADR 0122 rules 36.4 and 39); 0 when the clock is before the epoch, which
/// makes every checkpoint look ahead of it, so resolution stops with a clock skew.
pub(crate) fn local_clock() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The heap of the process, as the binary counts it (ADR 0061): the bytes it holds, and how to
/// set the ceiling past which an allocation fails. The library never touches the allocator.
#[derive(Clone, Copy)]
pub struct Heap<'a> {
    /// The bytes the process holds on the heap now.
    pub held: &'a (dyn Fn() -> usize + Sync),
    /// Sets the ceiling, in bytes; `usize::MAX` lifts it.
    pub ceiling: &'a (dyn Fn(usize) + Sync),
}

impl Heap<'static> {
    /// A heap that nothing counts: every run is within its memory budget.
    pub fn unmeasured() -> Heap<'static> {
        Heap {
            held: &|| 0,
            ceiling: &|_| {},
        }
    }
}

/// Runs the `una` command line. `args` starts with the program name; `stderr_is_terminal`
/// decides `--color auto`, since human diagnostics go to stderr (ADR 0015 D1).
///
/// A failure to write the output is reported as a usage error (exit code 2, ADR 0015 D1). A
/// panic anywhere in a command, on this thread or on a worker thread that hands it back when
/// joined, is an internal compiler error: U8001, exit code 3, and with `--json` one
/// `una.diagnostics/1` document on stdout (`spec/13 §5`, #376). Rust's own panic message stays
/// on stderr, as `spec/13 §5` allows for U8001.
pub fn run<I, T>(
    args: I,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    stderr_is_terminal: bool,
    environment: Environment<'_>,
    heap: Heap<'_>,
) -> ExitStatus
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    // The same binary invoked as `ita` is `una ita` (ADR 0122 rule 1): `ita lock` is
    // `una ita lock`.
    if invoked_as_ita(args.first()) {
        args.insert(1, OsString::from("ita"));
    }
    // `--json` is a global flag without a value, so its presence decides the shape of the
    // report even when parsing is what panicked.
    let json = args.iter().skip(1).any(|arg| arg == "--json");
    // What a panic report needs from the command line, read before the command runs; parsing
    // is guarded too, and a panic in it leaves both unknown.
    let parsed = std::panic::catch_unwind(|| parse_line(args.clone()).ok())
        .ok()
        .flatten();
    let command = parsed.as_ref().and_then(|cli| cli.command.as_ref());
    let location = panic_location(command);
    // With `--json`, a command's stdout is held until it returns, so a panic never leaves a
    // fragment of its document before the U8001 one (`spec/13 §5`: one document). No `--json`
    // command streams: `una run` puts the program's output inside `una.run/1`. The language
    // server speaks its protocol on stdout as it goes, so it is never held.
    let held = json && !matches!(command, Some(Command::Lsp));
    let guard = Guard {
        json,
        held,
        location,
    };
    // The command line is parsed once (#1894): building clap's command for every subcommand is
    // a fixed cost of each process. A line that did not parse here is parsed again inside the
    // guard, which reports its error, or a panic in parsing as U8001.
    guard.run(stdout, stderr, |stdout, stderr| {
        run_command(
            args,
            parsed,
            stdout,
            stderr,
            stderr_is_terminal,
            environment,
            heap,
        )
    })
}

/// Parses the command line. [`run`] calls it once for a line that parses, and once more for
/// one that does not, to report clap's error inside the guard (#1894).
fn parse_line(args: Vec<OsString>) -> Result<Cli, clap::Error> {
    #[cfg(test)]
    PARSES.with(|count| count.set(count.get() + 1));
    Cli::try_parse_from(args)
}

#[cfg(test)]
thread_local! {
    /// How many times [`parse_line`] ran on this thread, for the tests of [`run`].
    static PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether the program name, `argv[0]`, has the basename `ita`, or `ita.exe`.
fn invoked_as_ita(program: Option<&OsString>) -> bool {
    program
        .and_then(|program| Path::new(program).file_name())
        .is_some_and(|name| name == "ita" || name == "ita.exe")
}

/// Where U8001 for a panic goes (ADR 0022 rule 2): 1:1 of the `una.toml` of the package the
/// command checks, or of the checked file for a package of one file, as the commands locate
/// a diagnostic without a source position. A command without a package (`una explain`,
/// `una lsp`, `--version`), a path that names neither, or a panic while parsing the command
/// line leaves no file to point at: the location is then the empty `file` at 1:1, as
/// `root_location` already writes for a package without files, rather than a file that the
/// command never read.
fn panic_location(command: Option<&Command>) -> una_diag::Location {
    let root = match command {
        Some(
            Command::Check { path, .. }
            | Command::Core { path, .. }
            | Command::Report { path, .. }
            | Command::Obligations { path, .. }
            | Command::Fmt { path, .. }
            | Command::Migrate { path, .. }
            | Command::Fix { path, .. }
            | Command::Run { path, .. }
            | Command::Build { path, .. }
            | Command::Test { path, .. },
        ) => Some(path.clone()),
        Some(Command::Ita {
            command:
                ItaCommand::Lock { path, .. }
                | ItaCommand::Tree { path }
                | ItaCommand::Vendor { path }
                | ItaCommand::Verify { path }
                | ItaCommand::Remove { path, .. }
                | ItaCommand::Fetch { path }
                | ItaCommand::Publish { path, .. }
                | ItaCommand::Yank { path, .. }
                | ItaCommand::Audit { path }
                | ItaCommand::Bundle { path, .. }
                | ItaCommand::Unbundle { path, .. },
        }) => Some(path.clone()),
        // `una ita add` and `update` work on the package in the current directory.
        Some(Command::Ita {
            command: ItaCommand::Add { .. } | ItaCommand::Update { .. },
        }) => Some(".".into()),
        // `una trace` and `una contract` read the package in the current directory.
        Some(Command::Trace { .. } | Command::Contract { .. }) => Some(".".into()),
        _ => None,
    };
    let file = root.and_then(|root| {
        if root.is_file() {
            root.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        } else if root.join("ita.una").is_file() {
            Some("ita.una".to_owned())
        } else if root.join("una.toml").is_file() {
            Some("una.toml".to_owned())
        } else {
            None
        }
    });
    let position = una_diag::Position { line: 1, col: 1 };
    una_diag::Location {
        file: file.unwrap_or_default(),
        start: position,
        end: position,
    }
}

/// How [`run`] turns a panic into an internal compiler error (#376).
struct Guard {
    /// Whether the report is a JSON document.
    json: bool,
    /// Whether the command's stdout is held until it returns.
    held: bool,
    /// Where the U8001 diagnostic goes.
    location: una_diag::Location,
}

impl Guard {
    /// Runs `command`. On a panic, nothing it held reaches stdout, and U8001 is reported in its
    /// place, with exit code 3.
    fn run(
        &self,
        stdout: &mut dyn Write,
        stderr: &mut dyn Write,
        command: impl FnOnce(&mut dyn Write, &mut dyn Write) -> ExitStatus,
    ) -> ExitStatus {
        let mut buffer: Vec<u8> = Vec::new();
        // The writers are only written again on the panicking path to report it.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if self.held {
                command(&mut buffer, &mut *stderr)
            } else {
                command(&mut *stdout, &mut *stderr)
            }
        }));
        match outcome {
            Ok(status) => {
                if self.held
                    && stdout
                        .write_all(&buffer)
                        .and_then(|()| stdout.flush())
                        .is_err()
                {
                    // A failure to write the output (ADR 0015 D1): a closed stdout discards it
                    // without an error, as before (#259).
                    return ExitStatus::Usage;
                }
                status
            }
            Err(payload) => {
                let what = payload
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_owned())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "a panic without a message".to_owned());
                internal_error(self.json, &self.location, &what, stdout, stderr);
                ExitStatus::Internal
            }
        }
    }
}

/// Reports a panic as U8001 at `location`: one `una.diagnostics/1` document on stdout with
/// `--json`, or one line on stderr. A failure to write it changes nothing: the exit code says
/// it (#376).
fn internal_error(
    json: bool,
    location: &una_diag::Location,
    what: &str,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) {
    let message = format!("internal error: the compiler stopped on a bug: {what}");
    if json {
        let diagnostic = Diagnostic::error(Code::U8001, message.clone(), location.clone());
        match render_json(&[diagnostic]) {
            Ok(document) => {
                let _ = writeln!(stdout, "{document}").and_then(|()| stdout.flush());
            }
            Err(error) => {
                let _ = writeln!(
                    stderr,
                    "internal compiler error (U8001): {message}; rendering it as JSON failed: {error}"
                );
            }
        }
    } else {
        let _ = writeln!(stderr, "internal compiler error (U8001): {message}")
            .and_then(|()| stderr.flush());
    }
}

/// [`run`] without the guard: runs the command `parsed` holds, or, when `args` did not parse
/// in [`run`], parses them again for clap's error.
fn run_command(
    args: Vec<OsString>,
    parsed: Option<Cli>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    stderr_is_terminal: bool,
    environment: Environment<'_>,
    heap: Heap<'_>,
) -> ExitStatus {
    // The key of the proof cache is resolved when a command verifies, and not before: `una fmt`
    // never makes one (ADR 0064). A key file that exists but is refused turns the cache off,
    // with a note on stderr (#1164), except under `--json`, whose stderr stays empty
    // (`spec/13 §5`, ADR 0015 rule 3): the report's `tcb.proof_cache_key: null` is the signal.
    let solving = |seconds: u64, memory: u64, json: bool, stderr: &mut dyn Write| Solving {
        wallclock: Duration::from_secs(seconds),
        memory_mib: memory,
        proof_key: match proof_key::resolve(environment) {
            Ok(key) => key,
            Err(refused) => {
                if !json {
                    let hint = refused
                        .reason
                        .hint()
                        .map(|hint| format!(" ({hint})"))
                        .unwrap_or_default();
                    // A note that cannot be written changes nothing about the command.
                    let _ = writeln!(
                        stderr,
                        "note: the proof cache key `{}` is not used, since {}; the proof cache is off{hint}",
                        refused.shown, refused.reason
                    );
                }
                None
            }
        },
        itaoca: itaoca(environment),
    };
    let parsed = match parsed {
        Some(cli) => Ok(cli),
        None => parse_line(args),
    };
    let status = match parsed {
        Ok(cli) if cli.version => version_command(cli.json, stdout),
        // Neither a command nor `--version`: the usage, as clap prints it for a missing command.
        Ok(Cli { command: None, .. }) => {
            let usage = <Cli as clap::CommandFactory>::command().render_usage();
            write!(
                stderr,
                "error: a command is required\n\n{usage}\n\nFor more information, try '--help'.\n"
            )
            .map(|()| ExitStatus::Usage)
        }
        Ok(Cli {
            json,
            color,
            command: Some(command),
            ..
        }) => match command {
            Command::Check {
                path,
                tier,
                dump_core,
                solver_wallclock,
                solver_memory,
            } => {
                let options = CheckOptions {
                    tier: match tier {
                        TierArg::Zero => Tier::Zero,
                        TierArg::One => Tier::One,
                    },
                    json,
                    color: use_color(color, stderr_is_terminal),
                    solving: solving(solver_wallclock, solver_memory, json, stderr),
                };
                check_command(&path, options, dump_core.as_deref(), stdout, stderr)
            }
            Command::Core {
                path,
                with_rejected,
            } => core_command(
                &path,
                json,
                with_rejected,
                use_color(color, stderr_is_terminal),
                stdout,
                stderr,
            ),
            Command::Trace {
                requirement,
                solver_wallclock,
                solver_memory,
            } => {
                let options = trace::TraceOptions {
                    json,
                    color: use_color(color, stderr_is_terminal),
                    solving: solving(solver_wallclock, solver_memory, json, stderr),
                };
                trace::trace_command(&requirement, options, stdout, stderr)
            }
            Command::Report {
                path,
                solver_wallclock,
                solver_memory,
            } => {
                let options = report::ReportOptions {
                    json,
                    color: use_color(color, stderr_is_terminal),
                    solving: solving(solver_wallclock, solver_memory, json, stderr),
                    runtime_sha256: runtime_archive::reported_sha256(environment),
                };
                report::report_command(&path, options, stdout, stderr)
            }
            Command::Obligations {
                path,
                solver_wallclock,
                solver_memory,
            } => {
                let options = verification::ObligationsOptions {
                    json,
                    color: use_color(color, stderr_is_terminal),
                    solving: solving(solver_wallclock, solver_memory, json, stderr),
                };
                verification::obligations_command(&path, options, stdout, stderr)
            }
            Command::Fix {
                path,
                tier,
                dry_run,
                solver_wallclock,
                solver_memory,
            } => {
                let options = fix::FixOptions {
                    tier: match tier {
                        TierArg::Zero => Tier::Zero,
                        TierArg::One => Tier::One,
                    },
                    dry_run,
                    json,
                    color: use_color(color, stderr_is_terminal),
                    solving: solving(solver_wallclock, solver_memory, json, stderr),
                };
                fix::fix_command(&path, options, stdout, stderr)
            }
            Command::Fmt { path, check } if ita::is_project_file(&path) => {
                let output = ita::Output {
                    json,
                    color: use_color(color, stderr_is_terminal),
                    stdout,
                    stderr,
                };
                ita::fmt_command(&path, check, output)
            }
            Command::Migrate {
                path,
                dry_run,
                name,
            } => {
                let options = migrate::MigrateOptions {
                    dry_run,
                    name: name.as_deref(),
                    json,
                    color: use_color(color, stderr_is_terminal),
                };
                migrate::migrate_command(&path, options, stdout, stderr)
            }
            Command::Ita { command } => {
                let output = ita::Output {
                    json,
                    color: use_color(color, stderr_is_terminal),
                    stdout,
                    stderr,
                };
                let store = itaoca(environment);
                let clock = local_clock();
                match command {
                    ItaCommand::Tree { path } => ita::tree_command(&path, output),
                    ItaCommand::Remove { name, path } => ita::remove_command(&name, &path, output),
                    ItaCommand::Lock {
                        path,
                        check,
                        fresh,
                        accept_authority,
                        accept_trust,
                        accept_key,
                        dry_run,
                    } => ita::lock_command(
                        &path,
                        ita::LockOptions {
                            fresh,
                            accept_key: &accept_key,
                            itaoca: store.as_ref(),
                            now: clock,
                            ..ita::LockOptions::lock(
                                check,
                                dry_run,
                                &accept_authority,
                                &accept_trust,
                            )
                        },
                        output,
                    ),
                    ItaCommand::Update {
                        names,
                        cooldown,
                        now,
                        accept_authority,
                        accept_trust,
                        accept_key,
                        dry_run,
                    } => {
                        let options = ita::LockOptions {
                            command: "update",
                            cooldown: if now {
                                0
                            } else {
                                cooldown.map_or(una_project::resolve::COOLDOWN, |hours| {
                                    hours.saturating_mul(3600)
                                })
                            },
                            accept_key: &accept_key,
                            itaoca: store.as_ref(),
                            now: clock,
                            ..ita::LockOptions::lock(
                                false,
                                dry_run,
                                &accept_authority,
                                &accept_trust,
                            )
                        };
                        ita::update_command(&names, Path::new("."), options, output)
                    }
                    ItaCommand::Add {
                        name,
                        version,
                        local,
                        from,
                        now,
                        dry_run,
                    } => {
                        let options = ita::LockOptions {
                            command: "add",
                            cooldown: if now {
                                0
                            } else {
                                una_project::resolve::COOLDOWN
                            },
                            itaoca: store.as_ref(),
                            now: clock,
                            ..ita::LockOptions::lock(false, dry_run, &[], &[])
                        };
                        ita::add_command(
                            &name,
                            version.as_deref(),
                            local.as_deref(),
                            from.as_deref(),
                            Path::new("."),
                            options,
                            output,
                        )
                    }
                    ItaCommand::Fetch { path } => ita::fetch_command(&path, store.as_ref(), output),
                    ItaCommand::Vendor { path } => {
                        ita::vendor_command(&path, store.as_ref(), output)
                    }
                    ItaCommand::Verify { path } => {
                        ita::verify_command(&path, store.as_ref(), output)
                    }
                    ItaCommand::Publish { path } => {
                        // A refused proof cache key is a note on this command's stderr (#1164).
                        let solving = solving(
                            DEFAULT_WALLCLOCK.as_secs(),
                            crate::solver_child::DEFAULT_MEMORY_MIB,
                            json,
                            &mut *output.stderr,
                        );
                        let home = variable(environment, "HOME");
                        let keys = ita_registry::keys_dir(home.as_deref());
                        let git_variable = ["GIT_DIR", "GIT_WORK_TREE"]
                            .into_iter()
                            // Set at all, even to a value that is not UTF-8.
                            .find(|name| !matches!(environment(name), Ok(None)));
                        ita_registry::publish_command(
                            &path,
                            keys.as_deref(),
                            git_variable,
                            &solving,
                            output,
                        )
                    }
                    ItaCommand::Yank { version, path } => {
                        let store = itaoca(environment);
                        let home = variable(environment, "HOME");
                        let keys = ita_registry::keys_dir(home.as_deref());
                        ita_registry::yank_command(
                            &version,
                            &path,
                            keys.as_deref(),
                            store.as_ref(),
                            output,
                        )
                    }
                    ItaCommand::Audit { path } => {
                        ita_registry::audit_command(&path, itaoca(environment).as_ref(), output)
                    }
                    ItaCommand::Watch { owner, names, from } => {
                        let store = itaoca(environment);
                        ita_registry::watch_command(&owner, &names, &from, store.as_ref(), output)
                    }
                    ItaCommand::Bundle { file, path } => ita_registry::bundle_command(
                        &file,
                        &path,
                        itaoca(environment).as_ref(),
                        output,
                    ),
                    ItaCommand::Unbundle { file, path } => ita_registry::unbundle_command(
                        &file,
                        &path,
                        itaoca(environment).as_ref(),
                        output,
                    ),
                }
            }
            Command::Fmt { path, check } => {
                let options = FmtOptions {
                    check,
                    json,
                    color: use_color(color, stderr_is_terminal),
                };
                fmt_command(&path, options, stdout, stderr)
            }
            Command::Contract { name } => {
                let color = use_color(color, stderr_is_terminal);
                contract_command(&name, json, color, stdout, stderr)
            }
            Command::Run {
                path,
                grants,
                max_depth,
                max_memory,
                max_response,
                http_timeout,
                workers,
                task_depth,
                io_threads,
                file_threads,
                serve_threads,
                solver_wallclock,
                solver_memory,
                stop_after_iterations,
                virtual_time,
                cost_clock,
                deadline,
                stop_grace,
                trace,
                serve_script,
                serve_out,
            } => {
                let options = runtime::RunOptions {
                    grants: &grants,
                    max_depth,
                    tasks: runtime::TaskFlags {
                        workers,
                        task_depth,
                        io_threads,
                        file_threads,
                        serve_threads,
                    },
                    max_memory: max_memory.unwrap_or(una_interp::DEFAULT_MAX_MEMORY),
                    max_memory_given: max_memory.is_some(),
                    http: runtime::http_settings(max_response, http_timeout),
                    heap,
                    json,
                    color: use_color(color, stderr_is_terminal),
                    environment,
                    solving: solving(solver_wallclock, solver_memory, json, stderr),
                    stop_after: stop_after_iterations,
                    virtual_time,
                    cost_clock,
                    deadline,
                    stop_grace,
                    trace: trace.as_deref(),
                    serve_script: serve_script.as_deref(),
                    serve_out: serve_out.as_deref(),
                };
                runtime::run_command(&path, options, stdout, stderr)
            }
            Command::Build {
                path,
                out,
                debuggable,
                solver_wallclock,
                solver_memory,
            } => {
                let options = build::BuildOptions {
                    out,
                    debuggable,
                    json,
                    color: use_color(color, stderr_is_terminal),
                    solving: solving(solver_wallclock, solver_memory, json, stderr),
                    environment,
                };
                build::build_command(&path, &options, stdout, stderr)
            }
            Command::Test {
                path,
                max_depth,
                max_memory,
                workers,
                task_depth,
                io_threads,
                file_threads,
                solver_wallclock,
                solver_memory,
            } => {
                let options = runtime::RunOptions {
                    grants: &[],
                    max_depth,
                    tasks: runtime::TaskFlags {
                        workers,
                        task_depth,
                        io_threads,
                        file_threads,
                        serve_threads: None,
                    },
                    max_memory: max_memory.unwrap_or(una_interp::DEFAULT_MAX_MEMORY),
                    max_memory_given: max_memory.is_some(),
                    http: una_http::client::Settings::default(),
                    heap,
                    json,
                    color: use_color(color, stderr_is_terminal),
                    environment,
                    solving: solving(solver_wallclock, solver_memory, json, stderr),
                    stop_after: None,
                    // Time in a test is always virtual (ADR 0116 rule 19).
                    virtual_time: true,
                    cost_clock: false,
                    deadline: None,
                    stop_grace: una_interp::stopping::DEFAULT_GRACE_SECS,
                    trace: None,
                    serve_script: None,
                    serve_out: None,
                };
                runtime::test_command(&path, options, stdout, stderr)
            }
            Command::Explain { code } => explain_command(&code, json, stdout, stderr),
            Command::Lsp => {
                let solving = solving(
                    DEFAULT_WALLCLOCK.as_secs(),
                    crate::solver_child::DEFAULT_MEMORY_MIB,
                    false,
                    stderr,
                );
                lsp::serve(stdout, stderr, json, solving)
            }
        },
        // clap reports `--help` as an error that belongs on stdout with exit code 0.
        Err(error) => {
            let status = if error.exit_code() == 0 {
                ExitStatus::Success
            } else {
                ExitStatus::Usage
            };
            let rendered = error.render();
            let written = if error.use_stderr() {
                write!(stderr, "{rendered}")
            } else {
                write!(stdout, "{rendered}")
            };
            written.map(|()| status)
        }
    };
    status.unwrap_or(ExitStatus::Usage)
}

/// `una.version/1`, with keys in this order (ADR 0034 rule 4). `build` is present in a
/// development build only (ADR 0124 rule 19).
#[derive(serde::Serialize)]
struct VersionDocument {
    schema: &'static str,
    version: &'static str,
    editions: Vec<&'static str>,
    /// The editions served as previews, oldest first; `[]` when none (ADR 0121 rule 7).
    preview: Vec<&'static str>,
    z3: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    build: Option<&'static str>,
}

/// Whether this `una` is a development build (ADR 0124 rule 19): one built without the
/// release solver taken by digest, or without the embedded runtime (step 2 of that record). A
/// release build has both.
fn development_build() -> bool {
    !una_z3::BUILD.release || runtime_archive::SHA256.is_none()
}

/// The build record (ADR 0124 rule 20): what this `una` was built with, as one JSON document
/// with keys in a fixed order, which the solver child's handshake hashes and the report lists.
/// `runtime_sha256` is the digest of the embedded runtime (step 2), empty in a development
/// build. `musl_sha256s` (by file name, in byte order) and `libunwind_sha256` are the digests
/// of the musl inputs a Linux release carries (step 4a), empty on macOS and in a development
/// build.
#[derive(serde::Serialize)]
struct BuildRecord {
    schema: &'static str,
    version: &'static str,
    z3_version: &'static str,
    z3_commit: &'static str,
    z3_tree: &'static str,
    z3_source_sha256: &'static str,
    z3_library_sha256: &'static str,
    runtime_sha256: &'static str,
    musl_sha256s: std::collections::BTreeMap<&'static str, &'static str>,
    libunwind_sha256: &'static str,
    development: bool,
}

/// This `una`'s build record, as its JSON text.
pub fn build_record() -> String {
    let z3 = una_z3::BUILD;
    let record = BuildRecord {
        schema: "una.build-record/1",
        version: env!("CARGO_PKG_VERSION"),
        z3_version: z3.version,
        z3_commit: z3.commit,
        z3_tree: z3.tree,
        z3_source_sha256: z3.source_sha256,
        z3_library_sha256: z3.library_sha256,
        runtime_sha256: runtime_archive::SHA256.unwrap_or_default(),
        musl_sha256s: musl_inputs::musl_sha256s(),
        libunwind_sha256: musl_inputs::libunwind_sha256(),
        development: development_build(),
    };
    // INVARIANT: a struct of strings, a map of strings and a bool always serializes.
    serde_json::to_string(&record).unwrap_or_default()
}

/// `una --version` (`spec/13 §5`, #305): the version of `una`, the editions it serves and the
/// Z3 version built into it, with no commit hash, so builds stay reproducible; a development
/// build says so on its last line (ADR 0124 rule 19).
fn version_command(json: bool, stdout: &mut dyn Write) -> io::Result<ExitStatus> {
    let version = env!("CARGO_PKG_VERSION");
    let editions: Vec<&'static str> = una_project::Edition::ALL
        .iter()
        .map(|edition| edition.name())
        .collect();
    let preview: Vec<&'static str> = una_project::Edition::ALL
        .iter()
        .filter(|edition| edition.is_preview())
        .map(|edition| edition.name())
        .collect();
    let z3 = una_project::DEFAULT_SOLVER_VERSION;
    let build = development_build().then_some("development");
    if json {
        let document = VersionDocument {
            schema: "una.version/1",
            version,
            editions,
            preview,
            z3,
            build,
        };
        // INVARIANT: a struct of strings always serializes.
        let text = serde_json::to_string(&document).unwrap_or_default();
        writeln!(stdout, "{text}")?;
    } else {
        writeln!(stdout, "una {version}")?;
        writeln!(stdout, "editions: {}", editions.join(", "))?;
        if !preview.is_empty() {
            writeln!(stdout, "preview: {}", preview.join(", "))?;
        }
        writeln!(stdout, "z3: {z3}")?;
        if build.is_some() {
            writeln!(stdout, "development build")?;
        }
    }
    Ok(ExitStatus::Success)
}

/// The options of `una check`.
#[derive(Clone, Debug)]
struct CheckOptions {
    tier: Tier,
    json: bool,
    color: bool,
    solving: Solving,
}

/// `una check [path]`: prints the diagnostics of [`check`], as JSON on stdout (ADR 0012) or
/// for people on stderr (ADR 0015 D1).
fn check_command(
    path: &Path,
    options: CheckOptions,
    dump_core: Option<&str>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    let checked = match check_with(path, options.tier, &options.solving) {
        Ok(checked) => checked,
        // A usage error goes to stderr even with --json; it happens before the
        // command runs, like the usage errors clap reports.
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    // The Core IR of the module's declarations without diagnostics, on stdout.
    if let Some(module) = dump_core {
        match checked.program.modules.iter().find(|m| m.path == module) {
            Some(core) => write!(stdout, "{}", una_core::print_module(core))?,
            None => {
                writeln!(stderr, "error: the package has no module `{module}`")?;
                return Ok(ExitStatus::Usage);
            }
        }
    }
    report(
        &checked.diagnostics,
        &checked.sources,
        options.json,
        options.color,
        stdout,
        stderr,
    )
}

/// `una core [path]` (ADR 0083): checks the package at tier 0 and prints its Core IR, with
/// `--json` as one `una.core/1` document on stdout, otherwise in the text syntax of `spec/09`,
/// every module in turn. The diagnostics go to stderr, for people, in both modes, so stdout
/// holds the one document; the exit code follows them.
fn core_command(
    path: &Path,
    json: bool,
    with_rejected: bool,
    color: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    if with_rejected && !json {
        writeln!(
            stderr,
            "error: `--with-rejected` exports a `una.core/1` document, so it needs `--json`"
        )?;
        return Ok(ExitStatus::Usage);
    }
    let package = match load(path) {
        Ok(package) => package,
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let (manifest, _) = package::project(&package);
    // A program of several packages is a `una.core/2` document (ADR 0122 rule 59), which this
    // release does not write yet: the export is refused rather than written as one package.
    if !manifest.mounts.is_empty() {
        writeln!(
            stderr,
            "error: the package has dependencies, whose Core export (`una.core/2`) this release does not write yet"
        )?;
        return Ok(ExitStatus::Usage);
    }
    let (diagnostics, program, headers, of_lambdas, file_state, rejected) = on_large_stack(|| {
        let (analysis, rejected) = analyze_and(&package, None, with_rejected);
        (
            analysis.diagnostics,
            analysis.program,
            analysis.headers,
            analysis.of_lambdas,
            analysis.file_state,
            rejected,
        )
    });
    let complete = diagnostics.is_empty();
    let rejected = with_rejected.then(|| flow_rejected(rejected, &diagnostics, &package.sources));
    if json {
        match core_json::document(
            &program,
            &package.sources,
            &headers,
            &of_lambdas,
            &file_state,
            manifest.edition,
            complete,
            rejected.as_deref(),
        ) {
            Ok(document) => writeln!(stdout, "{document}")?,
            Err(error) => {
                writeln!(
                    stderr,
                    "internal compiler error (U8001): rendering Core IR as JSON failed: {error}"
                )?;
                return Ok(ExitStatus::Internal);
            }
        }
    } else {
        for module in &program.modules {
            write!(stdout, "{}", una_core::print_module(module))?;
        }
    }
    report(&diagnostics, &package.sources, false, color, stdout, stderr)
}

/// The rejected declarations that `una core --with-rejected` exports (ADR 0091 decision 3):
/// those with at least one diagnostic, every one of which, whatever pass reported it, is in the
/// `security` domain. A diagnostic belongs to the declaration whose span holds its primary
/// location. Each comes with its diagnostics, in output order.
fn flow_rejected<'d>(
    rejected: Vec<Rejected>,
    diagnostics: &'d [Diagnostic],
    sources: &SourceDb,
) -> Vec<(Rejected, Vec<&'d Diagnostic>)> {
    rejected
        .into_iter()
        .filter_map(|r| {
            let at = sources.location(r.span);
            let mut own: Vec<&Diagnostic> = diagnostics
                .iter()
                .filter(|d| {
                    d.primary.file == at.file
                        && d.primary.start >= at.start
                        && d.primary.end <= at.end
                })
                .collect();
            own.sort_by(|a, b| {
                (a.primary.start, a.code.as_str()).cmp(&(b.primary.start, b.code.as_str()))
            });
            let flow_only = !own.is_empty()
                && own
                    .iter()
                    .all(|d| d.code.domain() == una_diag::Domain::Security);
            flow_only.then_some((r, own))
        })
        .collect()
}

/// Prints diagnostics as JSON on stdout (ADR 0012) or for people on stderr (ADR 0015 D1), and
/// returns the exit status they imply.
fn report(
    diagnostics: &[Diagnostic],
    sources: &SourceDb,
    json: bool,
    color: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    // `spec/13 §5`: an internal compiler error (U8001) exits with 3, whatever else is reported.
    let status = if diagnostics.iter().any(|d| d.code == Code::U8001) {
        ExitStatus::Internal
    } else if diagnostics.is_empty() {
        ExitStatus::Success
    } else {
        ExitStatus::Errors
    };
    if json {
        match render_json(diagnostics) {
            Ok(document) => writeln!(stdout, "{document}")?,
            Err(error) => {
                writeln!(
                    stderr,
                    "internal compiler error (U8001): rendering diagnostics as JSON failed: {error}"
                )?;
                return Ok(ExitStatus::Internal);
            }
        }
        stdout.flush()?;
    } else {
        write!(
            stderr,
            "{}",
            render_human(diagnostics, &sources.texts(), color)
        )?;
        // ADR 0096 rule 6: how many diagnostics `una fix` would repair.
        let fixable = una_diag::fixable(diagnostics);
        if fixable > 0 {
            writeln!(
                stderr,
                "{fixable} of {} diagnostics have an edit that `una fix` applies (`una fix --dry-run` lists them)",
                diagnostics.len()
            )?;
        }
        stderr.flush()?;
    }
    Ok(status)
}

/// The options of `una fmt`.
#[derive(Clone, Copy, Debug)]
struct FmtOptions {
    check: bool,
    json: bool,
    color: bool,
}

/// `una fmt [path]` and `una fmt --check [path]` (`spec/13 §5`, ADR 0020).
///
/// Files with syntax errors are reported and left untouched; the others are
/// rewritten when they differ from their canonical form, each through a fresh temporary file
/// and a rename onto the file itself or, for a symbolic link that resolves inside the package,
/// onto its resolved target (a link leading outside it is refused), and nothing is printed on
/// success. `--check` reports U7001 instead of writing.
fn fmt_command(
    path: &Path,
    options: FmtOptions,
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
    let sources = &package.sources;
    let (manifest, _) = package::project(&package);
    let keywords = keyword_set(manifest.edition);
    let modules = with_keywords(&package.modules, keywords);
    let checks = on_large_stack(|| {
        let mut interner = Interner::new();
        check_files(sources, &modules, &mut interner)
    });
    let mut diagnostics = Vec::new();
    for checked in checks {
        let id = checked.id;
        diagnostics.extend(checked.syntax);
        let Some(canonical) = checked.canonical else {
            continue;
        };
        if options.check {
            diagnostics.extend(una_fmt::differences(sources, id, &canonical));
            continue;
        }
        let file = sources.file(id);
        if file.text() == canonical && file.first_crlf().is_none() {
            continue;
        }
        let Some(target) = package.disk_paths.get(&id) else {
            continue;
        };
        if let Err(error) = write_through_rename(&write_root(path), target, &canonical) {
            writeln!(
                stderr,
                "error: cannot write `{}`: {error}",
                target.display()
            )?;
            return Ok(ExitStatus::Usage);
        }
    }
    report(
        &diagnostics,
        sources,
        options.json,
        options.color,
        stdout,
        stderr,
    )
}

/// Writes `text` to `target`, which need not exist yet, through a fresh temporary file and a
/// rename, so a reader never sees a partial file (ADR 0122 rule 14). `root` is the package
/// directory the command works on; see [`write_through_rename`] for the rules on links.
pub(crate) fn write_through_rename_or_create(
    root: &Path,
    target: &Path,
    text: &str,
) -> io::Result<()> {
    una_diag::safe_write::write_atomically(root, target, text.as_bytes(), true, true)
}

/// Replaces the file `target` with `text` through a fresh temporary file and a rename, so an
/// interrupted write never leaves a truncated file, as [`una_diag::safe_write::write_atomically`]
/// writes it.
///
/// The temporary file is created next to the destination with `create_new` (`O_CREAT|O_EXCL`,
/// which never follows a symbolic link at its name), under a name unique to this process and
/// write, with mode 0600 on Unix until it takes the destination's permissions. Anything already
/// at that name, a planted link included, is never written through (#1160).
///
/// A symbolic link at `target` is written through onto the file it resolves to (review finding
/// R-024) only when that file, and the directory it lives in, resolve inside `root`. Otherwise
/// the write is refused, naming the link and where it leads: a cloned repository can choose a
/// target link as easily as a temporary one.
pub(crate) fn write_through_rename(root: &Path, target: &Path, text: &str) -> io::Result<()> {
    una_diag::safe_write::write_atomically(root, target, text.as_bytes(), false, true)
}

/// The package directory a command given `path` writes in: `path` itself, or the directory of a
/// single file.
pub(crate) fn write_root(path: &Path) -> PathBuf {
    if path.is_dir() {
        return path.to_path_buf();
    }
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// `una contract <qualified.name>` (`spec/13 §5`, ADR 0029): checks the package in the current
/// directory at tier 0 and prints the contract of the function, or the diagnostics as
/// `una check` does when there are any. An unknown name is a usage error.
fn contract_command(
    name: &str,
    json: bool,
    color: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    let package = match load(Path::new(".")) {
        Ok(package) => package,
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let analysis = on_large_stack(|| analyze(&package, None));
    // R-027, ADR 0036: the package's diagnostics, those of its project files included, come
    // before the name is resolved, so that an unknown name never hides them.
    if !analysis.diagnostics.is_empty() {
        return report(
            &analysis.diagnostics,
            &package.sources,
            json,
            color,
            stdout,
            stderr,
        );
    }
    let modules = contract::Modules {
        sources: &package.sources,
        interner: &analysis.interner,
        trees: &analysis.trees,
        mounts: &analysis.mounts,
    };
    let Some((module, decl, function)) = modules.function(name) else {
        writeln!(
            stderr,
            "error: the package has no function `{name}`; name it by module path and name, as in `events.checkin.check_in`"
        )?;
        return Ok(ExitStatus::Usage);
    };
    let function_name = analysis.interner.resolve(function.name.name);
    let header = analysis
        .headers
        .iter()
        .find(|h| h.module == module && h.name == function_name);
    let layout = verification::source_layout(&package);
    let contract = contract::contract(&modules, &module, decl, function, header, layout);
    if json {
        match serde_json::to_string(&contract) {
            Ok(document) => writeln!(stdout, "{document}")?,
            Err(error) => {
                writeln!(
                    stderr,
                    "internal compiler error (U8001): rendering the contract as JSON failed: {error}"
                )?;
                return Ok(ExitStatus::Internal);
            }
        }
    } else {
        write!(stdout, "{}", contract.human())?;
    }
    stdout.flush()?;
    Ok(ExitStatus::Success)
}

/// `una.explain/1`, with keys in this order (ADR 0034 rule 4).
#[derive(serde::Serialize)]
struct ExplainDocument {
    schema: &'static str,
    code: &'static str,
    domain: &'static str,
    meaning: &'static str,
    text: &'static str,
}

/// `una explain <code>`: prints the explanation of a registered code (`spec/13 §4`), or with
/// `--json` the document `una.explain/1`.
fn explain_command(
    code: &str,
    json: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    match code.parse::<Code>() {
        // `una.explain/1` with `--json`, as every command has a document.
        Ok(code) if json => {
            let document = ExplainDocument {
                schema: "una.explain/1",
                code: code.as_str(),
                domain: code.domain().as_str(),
                meaning: code.meaning(),
                text: una_explain::text(code),
            };
            match serde_json::to_string(&document) {
                Ok(text) => writeln!(stdout, "{text}")?,
                Err(error) => {
                    writeln!(
                        stderr,
                        "internal compiler error (U8001): rendering the explanation as JSON failed: {error}"
                    )?;
                    return Ok(ExitStatus::Internal);
                }
            }
            stdout.flush()?;
            Ok(ExitStatus::Success)
        }
        Ok(code) => {
            write!(stdout, "{}", una_explain::text(code))?;
            stdout.flush()?;
            Ok(ExitStatus::Success)
        }
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            Ok(ExitStatus::Usage)
        }
    }
}

/// Whether human output uses color, for a `--color` choice (`spec/13 §5`).
fn use_color(choice: ColorChoice, is_terminal: bool) -> bool {
    match choice {
        ColorChoice::Never => false,
        ColorChoice::Auto => is_terminal,
        ColorChoice::Always => true,
    }
}

/// `modules`, each read with `keywords`: the modules of one package, under its edition.
fn with_keywords(modules: &[FileId], keywords: KeywordSet) -> Vec<(FileId, KeywordSet)> {
    modules.iter().map(|&id| (id, keywords)).collect()
}

/// The keywords of a package's edition (`spec/01 §5`, `spec/17 §9`).
fn keyword_set(edition: una_project::Edition) -> KeywordSet {
    match edition {
        una_project::Edition::V0_1 => KeywordSet::V0_1,
        una_project::Edition::V0_2 => KeywordSet::V0_2,
        una_project::Edition::V0_3 => KeywordSet::V0_3,
        una_project::Edition::V0_4 => KeywordSet::V0_4,
        una_project::Edition::V0_5 => KeywordSet::V0_5,
    }
}

#[cfg(test)]
mod tests {
    use super::{ExitStatus, Guard, on_large_stack, panic_location};
    use crate::args::Command;

    /// The parses that `run` makes of `args`, with its exit status and stderr (#1894).
    fn parses_of(args: &[&str]) -> (usize, ExitStatus, String) {
        super::PARSES.with(|count| count.set(0));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = super::run(
            args.iter().copied(),
            &mut stdout,
            &mut stderr,
            false,
            &|_| Ok(None),
            super::Heap::unmeasured(),
        );
        let parses = super::PARSES.with(std::cell::Cell::get);
        (
            parses,
            status,
            String::from_utf8_lossy(&stderr).into_owned(),
        )
    }

    /// #1894: a command line that parses is parsed once per process, not once for the panic
    /// report and again for the command; one that does not parse is parsed a second time,
    /// inside the guard, and its error is clap's, as before.
    #[test]
    fn a_command_line_is_parsed_once_and_a_refused_one_reports_its_error() {
        let (parses, status, _) = parses_of(&["una", "--version"]);
        assert_eq!((parses, status), (1, ExitStatus::Success));
        let (parses, status, _) = parses_of(&["una", "explain", "U0001"]);
        assert_eq!((parses, status), (1, ExitStatus::Success));
        let (parses, status, stderr) = parses_of(&["una", "--no-such-flag"]);
        assert_eq!((parses, status), (2, ExitStatus::Usage), "{stderr}");
        assert!(stderr.contains("--no-such-flag"), "{stderr}");
    }

    /// ADR 0124 rule 20: the build record's keys, in their fixed order, with the musl inputs'
    /// digests after the runtime's (step 4a): an object by file name and libunwind's digest,
    /// both empty where `una` carries none (macOS, a development build).
    #[test]
    fn the_build_record_lists_its_keys_in_order_with_the_musl_inputs() {
        let record = super::build_record();
        let keys: Vec<&str> = [
            "\"schema\"",
            "\"z3_library_sha256\"",
            "\"runtime_sha256\"",
            "\"musl_sha256s\"",
            "\"libunwind_sha256\"",
            "\"development\"",
        ]
        .into_iter()
        .collect();
        let places: Vec<usize> = keys
            .iter()
            .map(|key| record.find(key).unwrap_or(usize::MAX))
            .collect();
        assert!(places.windows(2).all(|pair| pair[0] < pair[1]), "{record}");
        let value: serde_json::Value = serde_json::from_str(&record).unwrap();
        let musl = value["musl_sha256s"].as_object().unwrap();
        let libunwind = value["libunwind_sha256"].as_str().unwrap();
        if cfg!(una_musl_embedded) {
            assert_eq!(musl.len(), 6, "{record}");
            assert_eq!(libunwind.len(), 64, "{record}");
        } else {
            assert!(musl.is_empty(), "{record}");
            assert_eq!(libunwind, "", "{record}");
        }
    }

    /// The test-only hook of #376: a command that panics on the checker's own thread, as a bug
    /// in a pass would, joined and handed back as [`on_large_stack`] does.
    fn panicking(_: &mut dyn std::io::Write, _: &mut dyn std::io::Write) -> ExitStatus {
        on_large_stack(|| -> ExitStatus { panic!("a forced panic for #376") })
    }

    fn guard(json: bool, file: &str) -> Guard {
        let position = una_diag::Position { line: 1, col: 1 };
        Guard {
            json,
            held: json,
            location: una_diag::Location {
                file: file.to_owned(),
                start: position,
                end: position,
            },
        }
    }

    /// The one document on `stdout`, parsed.
    fn one_document(stdout: Vec<u8>) -> serde_json::Value {
        let stdout = String::from_utf8(stdout).unwrap();
        assert_eq!(stdout.lines().count(), 1, "{stdout}");
        serde_json::from_str(&stdout).unwrap()
    }

    #[test]
    fn a_panic_in_a_command_is_u8001_with_exit_code_3() {
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let status = guard(false, "una.toml").run(&mut stdout, &mut stderr, panicking);
        assert_eq!(status.code(), 3);
        assert!(stdout.is_empty());
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(
            stderr.starts_with(
                "internal compiler error (U8001): internal error: the compiler stopped on a bug: a forced panic for #376"
            ),
            "{stderr}"
        );
    }

    #[test]
    fn a_panic_with_json_prints_one_diagnostics_document_at_1_1_of_una_toml() {
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let status = guard(true, "una.toml").run(&mut stdout, &mut stderr, panicking);
        assert_eq!(status.code(), 3);
        // `spec/13 §5`: one JSON document on stdout; Rust's hook writes to the process's
        // stderr, not to ours.
        assert!(stderr.is_empty());
        let document = one_document(stdout);
        assert_eq!(document["schema"], "una.diagnostics/1");
        let diagnostics = document["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic["code"], "U8001");
        assert_eq!(diagnostic["severity"], "error");
        assert!(
            diagnostic["message"]
                .as_str()
                .unwrap()
                .ends_with("a forced panic for #376")
        );
        // ADR 0022 rule 2.
        assert_eq!(diagnostic["primary"]["file"], "una.toml");
        assert_eq!(
            diagnostic["primary"]["start"],
            serde_json::json!({"line": 1, "col": 1})
        );
    }

    #[test]
    fn a_panic_after_part_of_a_document_still_leaves_exactly_one_document() {
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let status = guard(true, "una.toml").run(&mut stdout, &mut stderr, |stdout, _| {
            let _ = write!(stdout, "{{\"schema\":\"una.report/2\",\"package\":");
            panic!("a forced panic in the middle of a document")
        });
        assert_eq!(status.code(), 3);
        let document = one_document(stdout);
        assert_eq!(document["schema"], "una.diagnostics/1");
        assert_eq!(document["diagnostics"][0]["code"], "U8001");
    }

    #[test]
    fn a_command_that_does_not_panic_keeps_its_status_and_its_output() {
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let status = guard(true, "una.toml").run(&mut stdout, &mut stderr, |stdout, _| {
            let _ = writeln!(
                stdout,
                "{{\"schema\":\"una.diagnostics/1\",\"diagnostics\":[],\"fixable\":0}}"
            );
            ExitStatus::Errors
        });
        assert_eq!(status.code(), 1);
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            "{\"schema\":\"una.diagnostics/1\",\"diagnostics\":[],\"fixable\":0}\n"
        );
        assert!(stderr.is_empty());
    }

    #[test]
    fn a_panic_is_located_at_the_package_file_or_nowhere() {
        let root = std::env::temp_dir().join(format!("una-376-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("una.toml"), "").unwrap();
        std::fs::write(root.join("src/one.una"), "").unwrap();
        let check = |path: std::path::PathBuf| Command::Check {
            path,
            tier: crate::args::TierArg::Zero,
            dump_core: None,
            solver_wallclock: 60,
            solver_memory: crate::solver_child::DEFAULT_MEMORY_MIB,
        };
        // A package: 1:1 of its `una.toml`; a file: 1:1 of that file; no package: nowhere.
        assert_eq!(panic_location(Some(&check(root.clone()))).file, "una.toml");
        assert_eq!(
            panic_location(Some(&check(root.join("src/one.una")))).file,
            "one.una"
        );
        assert_eq!(panic_location(Some(&check(root.join("missing")))).file, "");
        assert_eq!(panic_location(Some(&Command::Lsp)).file, "");
        assert_eq!(panic_location(None).file, "");
        let _ = std::fs::remove_dir_all(&root);
    }
}
