//! `una lsp` (ADR 0063): the language server over the checks of the CLI, so an editor shows
//! what `una check` reports, through the same lock and proof cache.

use std::collections::BTreeMap;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use una_lsp::{Checked, Workspace};

use crate::package::{load_in, load_with};
use crate::{ExitStatus, analyze, check_files, keyword_set, on_large_stack, verification};

/// The CLI's checks and formatter, as the server needs them.
pub(crate) struct CliWorkspace {
    /// How tier 1 runs the solver, with the user's key of the proof cache (ADR 0064).
    pub(crate) solving: verification::Solving,
}

impl Workspace for CliWorkspace {
    fn root(&self, file: &Path) -> PathBuf {
        file.ancestors()
            .skip(1)
            .find(|dir| dir.join("una.toml").is_file())
            .map_or_else(|| file.to_path_buf(), Path::to_path_buf)
    }

    fn check(
        &self,
        root: &Path,
        overlay: &BTreeMap<PathBuf, String>,
        solve: bool,
        cancel: &Arc<AtomicBool>,
    ) -> Checked {
        let Ok(package) = load_in(root, overlay, self.solving.itaoca.as_ref()) else {
            return Checked::default();
        };
        let mut analysis = on_large_stack(|| analyze(&package, None));
        let mut diagnostics = std::mem::take(&mut analysis.diagnostics);
        // Tier 1 runs once tier 0 reports nothing, as in `una check` (`spec/12 §3`).
        let mut verification = None;
        if solve && diagnostics.is_empty() {
            let found = verification::tier_one_cancellable(
                &package,
                &analysis.program,
                &self.solving,
                Some(Arc::clone(cancel)),
            );
            diagnostics.extend(found.diagnostics);
            verification = found.verification;
        }
        let mut checked = Checked {
            diagnostics,
            texts: package.sources.texts(),
            ..Checked::default()
        };
        index(
            &package.sources,
            &analysis,
            verification.as_ref(),
            verification::source_layout(&package),
            &mut checked,
        );
        checked
    }

    fn format(
        &self,
        root: &Path,
        file: &Path,
        overlay: &BTreeMap<PathBuf, String>,
    ) -> Option<String> {
        let package = load_with(root, overlay).ok()?;
        let id = package
            .disk_paths
            .iter()
            .find(|(_, path)| path.as_path() == file)
            .map(|(id, _)| *id)?;
        let (manifest, _) = crate::package::project(&package);
        let mut interner = una_syntax::Interner::new();
        let keywords = keyword_set(manifest.edition);
        check_files(
            &package.sources,
            &crate::with_keywords(&package.modules, keywords),
            &mut interner,
        )
        .into_iter()
        .find(|checked| checked.id == id)
        .and_then(|checked| checked.canonical)
    }
}

/// Serves the language server on the process's stdin and on `stdout`, until the client
/// sends `exit` or closes the input. A frame the server refuses is a U7016 diagnostic on
/// `stderr`, one line of text, or with `json` one `una.diagnostic/1` document a line, since
/// stdout carries the protocol (#2100, #2156, `spec/13 §5`). A session that reported one,
/// whether it dropped the frame and went on or read no further, exits with code 1, as for
/// any diagnostic with a code (ADR 0036); an error of reading the input or of writing the
/// output is returned, exit code 2.
pub(crate) fn serve(
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    json: bool,
    solving: verification::Solving,
) -> std::io::Result<ExitStatus> {
    let input = Box::new(BufReader::new(std::io::stdin()));
    let form = if json {
        una_lsp::LogForm::Json
    } else {
        una_lsp::LogForm::Text
    };
    let ended = una_lsp::serve(input, stdout, stderr, form, CliWorkspace { solving })?;
    Ok(if ended.reported() {
        ExitStatus::Errors
    } else {
        ExitStatus::Success
    })
}

/// What the editor asks about, from a check: definitions, hovers, the outline and the proof
/// counts (ADR 0063 rule 3).
fn index(
    sources: &una_syntax::SourceDb,
    analysis: &crate::Analysis,
    verification: Option<&verification::Verification>,
    layout: una_verify::SourceLayout,
    checked: &mut Checked,
) {
    use una_syntax::ast::DeclKind;

    let location = |span: una_syntax::Span| sources.location(span);
    for (from, to) in analysis
        .references
        .iter()
        .chain(&analysis.index.definitions)
    {
        checked.definitions.push((location(*from), location(*to)));
    }
    // A type's hover is printed when the editor shows it, not here (ADR 0080).
    for (span, ty) in &analysis.index.types {
        let ty = ty.clone();
        checked.hovers.push((
            location(*span),
            una_lsp::HoverText::Lazy(std::sync::Arc::new(move || format!("```una\n{ty}\n```"))),
        ));
    }
    // The contract of each function a call or a name refers to, as `una contract` prints it.
    let modules = crate::contract::Modules {
        sources,
        interner: &analysis.interner,
        trees: &analysis.trees,
        mounts: &analysis.mounts,
    };
    let mut contracts: BTreeMap<String, Option<String>> = BTreeMap::new();
    for (span, module, name) in &analysis.index.functions {
        let qualified = format!("{module}.{name}");
        let text = contracts
            .entry(qualified.clone())
            .or_insert_with(|| {
                let (module, decl, function) = modules.function(&qualified)?;
                let header = analysis
                    .headers
                    .iter()
                    .find(|h| h.module == module && h.name == name.as_str());
                let contract =
                    crate::contract::contract(&modules, &module, decl, function, header, layout);
                Some(format!("```una\n{}\n```", contract.lines().join("\n")))
            })
            .clone();
        if let Some(text) = text {
            checked.hovers.push((location(*span), text.into()));
        }
    }
    // Obligations: their status after tier 1, or their kind before it.
    match verification {
        Some(verification) => {
            for (obligation, outcome) in verification.obligations.iter().zip(&verification.outcomes)
            {
                checked.hovers.push((
                    location(obligation.source),
                    format!(
                        "**{}** {}: `{}`",
                        outcome.status.as_str(),
                        obligation.kind.as_str(),
                        obligation.goal
                    )
                    .into(),
                ));
            }
        }
        None => {
            for pending in &analysis.pending {
                checked.hovers.push((
                    pending.location.clone(),
                    format!("{} obligation, proven on save", pending.kind.as_str()).into(),
                ));
            }
        }
    }
    // The outline, and the proof counts of each function after tier 1.
    let mut counts: BTreeMap<String, [usize; 3]> = BTreeMap::new();
    if let Some(verification) = verification {
        for (obligation, outcome) in verification.obligations.iter().zip(&verification.outcomes) {
            let slot = match outcome.status {
                una_diag::ObligationStatus::Proven => 0,
                una_diag::ObligationStatus::Refuted => 1,
                una_diag::ObligationStatus::Unknown => 2,
            };
            if let Some(count) = counts
                .entry(obligation.declaration.clone())
                .or_default()
                .get_mut(slot)
            {
                *count += 1;
            }
        }
    }
    for (_, tree) in &analysis.trees {
        let path = modules.path(tree).unwrap_or_default();
        for decl in &tree.decls {
            let named =
                |ident: &una_syntax::ast::Ident| analysis.interner.resolve(ident.name).to_owned();
            // LSP `SymbolKind`: Class 5, Method 6, Enum 10, Interface 11, Function 12,
            // Constant 14, Object 19, Struct 23, Event 24, TypeParameter 26.
            let (name, selection, kind) = match &decl.kind {
                // A unit of measure (ADR 0104): no LSP kind names it; a type-level name.
                DeclKind::Measure(measure) => (named(&measure.name), measure.name.span, 26),
                DeclKind::Kind(kind) => (named(&kind.name), kind.name.span, 23),
                DeclKind::Predicate(predicate) => (named(&predicate.name), predicate.name.span, 6),
                DeclKind::Function(function) => (named(&function.name), function.name.span, 12),
                DeclKind::Capability(capability) => {
                    (named(&capability.name), capability.name.span, 11)
                }
                DeclKind::Handler(handler) => (named(&handler.name), handler.name.span, 5),
                DeclKind::Policy(policy) => (named(&policy.name), policy.name.span, 19),
                DeclKind::Requirement(requirement) => {
                    (named(&requirement.id), requirement.id.span, 14)
                }
                DeclKind::Example(example) => (
                    sources
                        .file(example.name.span.file)
                        .text()
                        .get(example.name.span.start..example.name.span.end)
                        .unwrap_or("example")
                        .to_owned(),
                    example.name.span,
                    24,
                ),
            };
            if let DeclKind::Function(_) = &decl.kind
                && let Some([proven, refuted, unknown]) = counts.get(&format!("{path}.{name}"))
            {
                checked.lenses.push((
                    location(selection),
                    format!("{proven} proven, {refuted} refuted, {unknown} unknown"),
                ));
            }
            checked.symbols.push(una_lsp::Symbol {
                name,
                kind,
                range: location(decl.span),
                selection: location(selection),
            });
        }
    }
}
