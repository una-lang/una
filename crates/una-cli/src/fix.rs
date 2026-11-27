//! `una fix [path]`: applies the mechanical repairs that `una check` reports (ADR 0084).
//!
//! Edits are applied in memory, in rounds that each check the package again at tier 0, where
//! every mechanical edit but one comes from. A round whose tier-0 check is clean also checks
//! tier 1, when the requested tier is 1, for the edits of callees' conditions (ADR 0084 and ADR
//! 0092, amendments of 2026-10-14). Then every module that parses is formatted, each changed
//! file is written once, and the package is checked at the requested tier for what remains.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use una_diag::{Code, Diagnostic, Domain, Edit, ObligationKind, Position, Repair, in_output_order};
use una_syntax::{Interner, SourceDb};

use crate::package::{self, Package, load_with};
use crate::verification::{self, Solving};
use crate::{ExitStatus, Tier, analyze, check_files, keyword_set, on_large_stack};

/// The most rounds one run applies (ADR 0084 rule 3).
const MAX_ROUNDS: usize = 16;

/// The schema of `una fix --json`.
const FIX_SCHEMA: &str = "una.fix/1";

/// The options of `una fix`.
pub(crate) struct FixOptions {
    /// The tier of the final check.
    pub(crate) tier: Tier,
    /// Report what would be applied, and write nothing.
    pub(crate) dry_run: bool,
    pub(crate) json: bool,
    pub(crate) color: bool,
    pub(crate) solving: Solving,
}

/// Why rounds stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Stopped {
    /// A round found no edit to apply.
    Done,
    /// The texts returned to a state already seen.
    Cycle,
    /// [`MAX_ROUNDS`] rounds ran.
    Limit,
}

/// An applied edit, with its positions in the text of its round.
#[derive(Clone, Debug, Serialize)]
struct Applied {
    round: usize,
    file: String,
    code: Code,
    description: String,
    start: Position,
    end: Position,
    replacement: String,
}

/// A first repair with an edit that was not applied, in the final text.
#[derive(Clone, Debug, Serialize)]
struct Skipped {
    file: String,
    code: Code,
    start: Position,
    description: String,
    reason: &'static str,
}

/// `una.fix/1`, with keys in the order of ADR 0084 rule 6.
#[derive(Serialize)]
struct FixDocument<'a> {
    schema: &'static str,
    dry_run: bool,
    rounds: usize,
    stopped: Stopped,
    applied: &'a [Applied],
    skipped: &'a [Skipped],
    formatted: &'a [String],
    diagnostics: Vec<&'a Diagnostic>,
}

/// One module of the package as `una fix` holds it.
struct Module {
    /// Where it is on disk, which is also its key in a loading overlay.
    disk: PathBuf,
    /// Its decoded text, as the rounds leave it.
    text: String,
    /// Whether the file on disk differs from its decoded text: `\r\n` line ends.
    crlf: bool,
    /// Whether it holds invalid UTF-8; such a file is never written.
    invalid: bool,
}

/// `una fix [path]` (`spec/13 §5`, ADR 0084).
pub(crate) fn fix_command(
    path: &Path,
    options: FixOptions,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    let package = match package::load(path) {
        Ok(package) => package,
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let mut modules = modules_of(&package);
    let original: BTreeMap<String, String> = modules
        .iter()
        .map(|(name, module)| (name.clone(), module.text.clone()))
        .collect();
    let mut applied = Vec::new();
    let mut seen = BTreeSet::from([texts(&modules)]);
    let mut rounds = 0;
    // What the last round's tier 1 checked, every file of the package, and its diagnostics:
    // the final check reuses them when every file is the same, rather than solve again.
    let mut checked: Option<(Files, Vec<Diagnostic>)> = None;
    let stopped = loop {
        let Some(package) = reload(path, &modules, stderr)? else {
            return Ok(ExitStatus::Usage);
        };
        let (diagnostics, program) = on_large_stack(|| {
            let analysis = analyze(&package, None);
            (analysis.diagnostics, analysis.program)
        });
        let mut taken = take_round(&diagnostics, &package.sources, &modules);
        // ADR 0084, amendment of 2026-10-14: a clean tier 0 checks tier 1, whose only
        // candidates are the edits of refuted `effect_condition` obligations. The report gives
        // none while an obligation is unknown (ADR 0092's condition (h)), so such a round takes
        // no edit and the rounds stop. A program for which the checker computed no such edit
        // has no candidate there, so its tier 1 is left to the final check.
        if diagnostics.is_empty()
            && options.tier == Tier::One
            && (!program.condition_edits.is_empty() || restores_locks(&program))
        {
            let found = verification::tier_one(&package, &program, &options.solving).diagnostics;
            let candidates: Vec<Diagnostic> = found
                .iter()
                .filter(|diagnostic| tier_one_candidate(diagnostic))
                .cloned()
                .collect();
            taken = take_round(&candidates, &package.sources, &modules);
            checked = snapshot(&package.sources).map(|files| (files, found));
        }
        if taken.is_empty() {
            break Stopped::Done;
        }
        rounds += 1;
        for (name, edits) in taken {
            let Some(module) = modules.get_mut(&name) else {
                continue;
            };
            // From the end of the text, so that earlier offsets stay valid.
            for (start, end, edit, code, description) in edits.into_iter().rev() {
                module.text.replace_range(start..end, &edit.replacement);
                applied.push(Applied {
                    round: rounds,
                    file: name.clone(),
                    code,
                    description,
                    start: edit.location.start,
                    end: edit.location.end,
                    replacement: edit.replacement,
                });
            }
        }
        if !seen.insert(texts(&modules)) {
            break Stopped::Cycle;
        }
        if rounds == MAX_ROUNDS {
            break Stopped::Limit;
        }
    };
    // Within a round, edits were applied from the end of each file; report them in text order.
    applied.sort_by(|a, b| {
        (a.round, &a.file, a.start.line, a.start.col).cmp(&(
            b.round,
            &b.file,
            b.start.line,
            b.start.col,
        ))
    });
    let Some(formatted) = format_modules(path, &mut modules, stderr)? else {
        return Ok(ExitStatus::Usage);
    };
    if !options.dry_run {
        for (name, module) in &modules {
            if module.invalid || original.get(name) == Some(&module.text) && !module.crlf {
                continue;
            }
            if let Err(error) =
                crate::write_through_rename(&crate::write_root(path), &module.disk, &module.text)
            {
                writeln!(
                    stderr,
                    "error: cannot write `{}`: {error}",
                    module.disk.display()
                )?;
                return Ok(ExitStatus::Usage);
            }
        }
    }
    let Some(package) = reload(path, &modules, stderr)? else {
        return Ok(ExitStatus::Usage);
    };
    let (mut diagnostics, program) = on_large_stack(|| {
        let analysis = analyze(&package, None);
        (analysis.diagnostics, analysis.program)
    });
    if options.tier == Tier::One && diagnostics.is_empty() {
        diagnostics = match snapshot(&package.sources).and_then(|now| reused(checked, &now)) {
            Some(found) => found,
            None => verification::tier_one(&package, &program, &options.solving).diagnostics,
        };
    }
    let mut skipped = skipped_repairs(&diagnostics, &modules, stopped);
    // ADR 0137 rule 11: the `let`s of doc comments' code are listed with the rest, in text order.
    let left = doc_comment_lets(&package, &modules);
    if !left.is_empty() {
        skipped.extend(left);
        skipped.sort_by(|a, b| {
            (&a.file, a.start.line, a.start.col).cmp(&(&b.file, b.start.line, b.start.col))
        });
    }
    if options.json {
        let document = FixDocument {
            schema: FIX_SCHEMA,
            dry_run: options.dry_run,
            rounds,
            stopped,
            applied: &applied,
            skipped: &skipped,
            formatted: &formatted,
            diagnostics: in_output_order(&diagnostics),
        };
        match serde_json::to_string(&document) {
            Ok(text) => writeln!(stdout, "{text}")?,
            Err(error) => {
                writeln!(
                    stderr,
                    "internal compiler error (U8001): rendering the output as JSON failed: {error}"
                )?;
                return Ok(ExitStatus::Internal);
            }
        }
        stdout.flush()?;
        return Ok(status(&diagnostics));
    }
    for edit in &applied {
        writeln!(
            stdout,
            "fixed {}:{}:{} {} {}",
            edit.file, edit.start.line, edit.start.col, edit.code, edit.description
        )?;
    }
    for skip in &skipped {
        writeln!(
            stdout,
            "skipped {}:{}:{} {} {}: {}",
            skip.file, skip.start.line, skip.start.col, skip.code, skip.description, skip.reason
        )?;
    }
    match stopped {
        Stopped::Done => {}
        Stopped::Cycle => writeln!(stdout, "stopped: the edits returned to an earlier text")?,
        Stopped::Limit => writeln!(stdout, "stopped: {MAX_ROUNDS} rounds applied")?,
    }
    stdout.flush()?;
    crate::report(
        &diagnostics,
        &package.sources,
        false,
        options.color,
        stdout,
        stderr,
    )
}

/// The modules of `package`, by their path relative to the package root.
fn modules_of(package: &Package) -> BTreeMap<String, Module> {
    package
        .modules
        .iter()
        .filter_map(|&id| {
            let file = package.sources.file(id);
            let disk = package.disk_paths.get(&id)?.clone();
            let invalid = file.text().char_indices().any(|(offset, c)| {
                c == char::REPLACEMENT_CHARACTER && file.is_invalid_sequence(offset)
            });
            Some((
                file.path().to_owned(),
                Module {
                    disk,
                    text: file.text().to_owned(),
                    crlf: file.first_crlf().is_some(),
                    invalid,
                },
            ))
        })
        .collect()
}

/// Every module's text, in path order: one state of the rounds.
fn texts(modules: &BTreeMap<String, Module>) -> Vec<String> {
    modules.values().map(|module| module.text.clone()).collect()
}

/// The package at `path` with the modules' current texts. A file that cannot be read any
/// more is a usage error, reported on `stderr`.
fn reload(
    path: &Path,
    modules: &BTreeMap<String, Module>,
    stderr: &mut dyn Write,
) -> io::Result<Option<Package>> {
    let overlay: BTreeMap<PathBuf, String> = modules
        .values()
        .filter(|module| !module.invalid)
        .map(|module| (module.disk.clone(), module.text.clone()))
        .collect();
    match load_with(path, &overlay) {
        Ok(package) => Ok(Some(package)),
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            Ok(None)
        }
    }
}

/// Every file of a package as it was loaded, by path: the modules, `una.toml` and
/// `verification.lock`.
type Files = BTreeMap<String, String>;

/// Every file of the package by its content, when its decoded text is its bytes: no file holds
/// invalid UTF-8 or a `\r\n`, the two things decoding changes (ADR 0015 D2). `None` otherwise,
/// and then no tier-1 result is reused.
fn snapshot(sources: &SourceDb) -> Option<Files> {
    sources
        .files()
        .map(|(_, file)| {
            let invalid = file.text().char_indices().any(|(offset, c)| {
                c == char::REPLACEMENT_CHARACTER && file.is_invalid_sequence(offset)
            });
            (!invalid && file.first_crlf().is_none())
                .then(|| (file.path().to_owned(), file.text().to_owned()))
        })
        .collect()
}

/// The diagnostics of a round's tier 1, when the final check is of the same package: every file
/// byte for byte the same, `una.toml` and `verification.lock` included (ADR 0084, amendment of
/// 2026-10-14). Otherwise the final check solves again.
fn reused(checked: Option<(Files, Vec<Diagnostic>)>, now: &Files) -> Option<Vec<Diagnostic>> {
    checked.and_then(|(files, found)| (&files == now).then_some(found))
}

/// Whether a diagnostic of tier 1 may give `una fix` an edit: a refuted `effect_condition`
/// obligation (ADR 0084, amendment of 2026-10-14) or a refuted `contract_lock` obligation
/// (amendment of 2026-10-15), the only kinds of tier 1 whose repair carries one. Another kind
/// needs its own amendment first.
fn tier_one_candidate(diagnostic: &Diagnostic) -> bool {
    let kind = diagnostic
        .obligation
        .as_ref()
        .map(|obligation| obligation.kind);
    // ADR 0084, amendment of 2026-10-15: a refuted `contract_lock` obligation is U3023, and
    // its restoration a candidate too; an unknown obligation (U5002) never is.
    (diagnostic.code == Code::U5001 && kind == Some(ObligationKind::EffectCondition))
        || (diagnostic.code == Code::U3023 && kind == Some(ObligationKind::ContractLock))
}

/// Whether the checker computed an edit that restores a locked condition in some header (ADR
/// 0092, amendment of 2026-10-15): such a program has a tier-1 candidate to look for.
fn restores_locks(program: &una_core::Program) -> bool {
    program.modules.iter().any(|module| {
        module.decls.iter().any(|decl| match decl {
            una_core::Decl::Fun(fun) => fun.locks.iter().any(|lock| lock.edit.is_some()),
            _ => false,
        })
    })
}

/// The first repair of `diagnostic`, in the order `spec/13 §1` prints them: by escape cost,
/// then rank.
fn first_repair(diagnostic: &Diagnostic) -> Option<&Repair> {
    diagnostic
        .repairs
        .iter()
        .min_by_key(|repair| repair.order_key())
}

/// Why a first repair with an edit is not applied; `None` when it is a candidate.
fn ineligible(
    diagnostic: &Diagnostic,
    repair: &Repair,
    edit: &Edit,
    modules: &BTreeMap<String, Module>,
    unparsed: &BTreeSet<&str>,
) -> Option<&'static str> {
    if repair.escape.is_some() {
        return Some("escape cost");
    }
    if repair.widens() || !keeps_ceiling(diagnostic.code, &edit.replacement) {
        return Some("widens authority");
    }
    match modules.get(&edit.location.file) {
        None => Some("not a module"),
        Some(module) if module.invalid => Some("invalid UTF-8"),
        // The final `una fmt` step formats every file that parses.
        Some(_) if diagnostic.code == Code::U7001 => Some("formatting"),
        // ADR 0137 rule 11: the rewrite of `let` is left in a file that does not parse, and
        // listed; once the file's syntax errors are repaired, a later round takes it.
        Some(_)
            if diagnostic.code == Code::U7007 && unparsed.contains(edit.location.file.as_str()) =>
        {
            Some("does not parse")
        }
        Some(_) => None,
    }
}

/// The files where `diagnostics` hold a syntax error (`spec/13 §4`, domain `syntax`): the files
/// that do not parse, where the rewrite of `let` waits (ADR 0137 rule 11).
fn unparsed(diagnostics: &[Diagnostic]) -> BTreeSet<&str> {
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.domain() == Domain::Syntax)
        .map(|diagnostic| diagnostic.primary.file.as_str())
        .collect()
}

/// Each `let` in the code of a doc comment of the package's modules, between backticks, in a
/// package of edition 0.5 or later (ADR 0137 rule 11): `una fix` cannot rewrite it exactly, since
/// a doc comment's code is text no check reads, so it lists the site and leaves it. Its
/// description is the rewrite a binding there would take. Linear in the size of the modules.
fn doc_comment_lets(package: &Package, modules: &BTreeMap<String, Module>) -> Vec<Skipped> {
    let (manifest, _) = package::project(package);
    if manifest.edition < una_project::Edition::V0_5 {
        return Vec::new();
    }
    let keywords = keyword_set(manifest.edition);
    let mut interner = Interner::new();
    let mut sites = Vec::new();
    for &id in &package.modules {
        let file = package.sources.file(id);
        if modules.get(file.path()).is_none_or(|module| module.invalid) {
            continue;
        }
        let lexed = una_syntax::lex_in(file, id, &mut interner, keywords);
        for comment in lexed.comments.iter().filter(|comment| comment.doc) {
            let text = file
                .text()
                .get(comment.span.start..comment.span.end)
                .unwrap_or("");
            for (offset, edit) in lets_in_code(text) {
                sites.push(Skipped {
                    file: file.path().to_owned(),
                    code: Code::U7007,
                    start: file.position(comment.span.start + offset),
                    description: if edit { "write `var`" } else { "write `const`" }.to_owned(),
                    reason: "doc comment",
                });
            }
        }
    }
    sites
}

/// The offset of each word `let` of `text` inside a backtick span, and whether the word `edit`
/// follows it after spaces, as in `let edit x`.
fn lets_in_code(text: &str) -> Vec<(usize, bool)> {
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut in_code = false;
    let mut previous: Option<u8> = None;
    for (at, &c) in bytes.iter().enumerate() {
        if c == b'`' {
            in_code = !in_code;
        } else if in_code
            && !previous.is_some_and(word)
            && text.get(at..).is_some_and(|rest| rest.starts_with("let"))
            && !bytes.get(at + 3).is_some_and(|&next| word(next))
        {
            let rest = text.get(at + 3..).unwrap_or("");
            let after = rest.trim_start_matches(' ');
            let edit = after.len() < rest.len()
                && after.starts_with("edit")
                && !after.as_bytes().get(4).is_some_and(|&next| word(next));
            found.push((at, edit));
        }
        previous = Some(c);
    }
    found
}

/// Whether an edit of a diagnostic of `code` that writes `replacement` grants a boundary
/// nothing beyond what the checker bounded (ADR 0093 rule 6, ADR 0084). The checker gives a
/// boundary diagnostic an edit only for a U3020 whose grant lies within the ceiling, and that
/// edit is one narrowed rebinding, ` with @x = @y[ops]` or `, @x = @y[ops]`. Any other edit of
/// U3020, U3021 or U3024 would grant a whole binding, a handler or an escape to code the
/// consumer does not trust, so it is never applied, whatever a later checker offers. The
/// ceiling itself is in `una.toml`, which `una fix` never writes. Linear in the replacement.
fn keeps_ceiling(code: Code, replacement: &str) -> bool {
    if !matches!(code, Code::U3020 | Code::U3021 | Code::U3024) {
        return true;
    }
    code == Code::U3020 && narrowed_grant(replacement)
}

/// Whether `text` is exactly one narrowed rebinding added to a statement: ` with @x = @y[ops]`
/// or `, @x = @y[ops]`, with names and at least one operation, and nothing else.
fn narrowed_grant(text: &str) -> bool {
    let Some(binding) = text
        .strip_prefix(" with ")
        .or_else(|| text.strip_prefix(", "))
    else {
        return false;
    };
    let name = |part: &str| {
        part.strip_prefix('@').is_some_and(|rest| {
            !rest.is_empty() && rest.chars().all(|c| c.is_alphanumeric() || c == '_')
        })
    };
    let Some((target, source)) = binding.split_once(" = ") else {
        return false;
    };
    let Some((held, ops)) = source
        .strip_suffix(']')
        .and_then(|rest| rest.split_once('['))
    else {
        return false;
    };
    name(target)
        && name(held)
        && ops
            .split(", ")
            .all(|op| !op.is_empty() && op.chars().all(|c| c.is_alphanumeric() || c == '_'))
}

/// One candidate edit of a round: its byte range in the current text, the edit, the code of
/// its diagnostic and the repair's description.
type Candidate = (usize, usize, Edit, Code, String);

/// The edits of one round, by file, in text order: the candidates that conflict with none
/// taken before them (ADR 0084 rule 3).
fn take_round(
    diagnostics: &[Diagnostic],
    sources: &SourceDb,
    modules: &BTreeMap<String, Module>,
) -> BTreeMap<String, Vec<Candidate>> {
    let mut by_file: BTreeMap<String, Vec<(Candidate, usize)>> = BTreeMap::new();
    let unparsed = unparsed(diagnostics);
    for (index, diagnostic) in in_output_order(diagnostics).into_iter().enumerate() {
        let Some(repair) = first_repair(diagnostic) else {
            continue;
        };
        let Some(edit) = &repair.edit else {
            continue;
        };
        if ineligible(diagnostic, repair, edit, modules, &unparsed).is_some() {
            continue;
        }
        let Some((start, end)) = range(sources, edit) else {
            continue;
        };
        by_file
            .entry(edit.location.file.clone())
            .or_default()
            .push((
                (
                    start,
                    end,
                    edit.clone(),
                    diagnostic.code,
                    repair.description.clone(),
                ),
                index,
            ));
    }
    by_file
        .into_iter()
        .map(|(file, mut candidates)| {
            candidates.sort_by_key(|((start, end, ..), index)| (*start, *end, *index));
            let mut taken: Vec<Candidate> = Vec::new();
            for (candidate, _) in candidates {
                if taken.iter().any(|t| same(t, &candidate)) {
                    continue;
                }
                if taken.iter().all(|t| !conflict(t, &candidate)) {
                    taken.push(candidate);
                }
            }
            (file, taken)
        })
        .collect()
}

/// Whether two edits are the same: one range and one replacement.
fn same(a: &Candidate, b: &Candidate) -> bool {
    a.0 == b.0 && a.1 == b.1 && a.2.replacement == b.2.replacement
}

/// Whether two edits conflict: their ranges overlap, or they touch at a point where either
/// inserts.
fn conflict(a: &Candidate, b: &Candidate) -> bool {
    let (a_start, a_end) = (a.0, a.1);
    let (b_start, b_end) = (b.0, b.1);
    if a_start < b_end && b_start < a_end {
        return true;
    }
    let touch = a_end == b_start || b_end == a_start;
    touch && (a_start == a_end || b_start == b_end)
}

/// The byte range in the current text of `edit`, whose positions count Unicode scalar values
/// (`spec/13 §1`).
fn range(sources: &SourceDb, edit: &Edit) -> Option<(usize, usize)> {
    let (_, file) = sources
        .files()
        .find(|(_, file)| file.path() == edit.location.file)?;
    let start = offset(file.text(), file.line_starts(), edit.location.start)?;
    let end = offset(file.text(), file.line_starts(), edit.location.end)?;
    (start <= end).then_some((start, end))
}

/// The byte offset of `position` in `text`, or `None` when the text has no such position.
fn offset(text: &str, line_starts: &[usize], position: Position) -> Option<usize> {
    let line = usize::try_from(position.line.checked_sub(1)?).ok()?;
    let col = usize::try_from(position.col.checked_sub(1)?).ok()?;
    let start = *line_starts.get(line)?;
    let end = line_starts.get(line + 1).copied().unwrap_or(text.len());
    text.get(start..end)?
        .char_indices()
        .map(|(index, _)| start + index)
        .chain(std::iter::once(end))
        .nth(col)
}

/// Formats every module that parses (ADR 0084 rule 4), and returns the paths of those it
/// changed. `None` when the package cannot be loaded any more.
fn format_modules(
    path: &Path,
    modules: &mut BTreeMap<String, Module>,
    stderr: &mut dyn Write,
) -> io::Result<Option<Vec<String>>> {
    let Some(package) = reload(path, modules, stderr)? else {
        return Ok(None);
    };
    let (manifest, _) = package::project(&package);
    let keywords = keyword_set(manifest.edition);
    let checks = on_large_stack(|| {
        let mut interner = Interner::new();
        check_files(
            &package.sources,
            &crate::with_keywords(&package.modules, keywords),
            &mut interner,
        )
    });
    let mut formatted = Vec::new();
    for checked in checks {
        let Some(canonical) = checked.canonical else {
            continue;
        };
        let name = package.sources.file(checked.id).path();
        let Some(module) = modules.get_mut(name) else {
            continue;
        };
        if module.invalid {
            continue;
        }
        if module.text != canonical || module.crlf {
            module.text = canonical;
            formatted.push(name.to_owned());
        }
    }
    Ok(Some(formatted))
}

/// The first repairs with an edit that the final check still reports and that `una fix` did
/// not apply, with the reason; an edit that is still a candidate waited on a conflict when the
/// rounds stopped.
fn skipped_repairs(
    diagnostics: &[Diagnostic],
    modules: &BTreeMap<String, Module>,
    stopped: Stopped,
) -> Vec<Skipped> {
    let unparsed = unparsed(diagnostics);
    in_output_order(diagnostics)
        .into_iter()
        .filter_map(|diagnostic| {
            let repair = first_repair(diagnostic)?;
            let edit = repair.edit.as_ref()?;
            let reason = match ineligible(diagnostic, repair, edit, modules, &unparsed) {
                // A formatting edit left after the final format belongs to a file that does
                // not parse; its syntax errors say why.
                Some("formatting") => return None,
                Some(reason) => reason,
                None if stopped == Stopped::Done => return None,
                None => "round limit",
            };
            Some(Skipped {
                file: edit.location.file.clone(),
                code: diagnostic.code,
                description: repair.description.clone(),
                start: edit.location.start,
                reason,
            })
        })
        .collect()
}

/// The exit status that the remaining diagnostics imply (ADR 0036), as `una check` exits.
fn status(diagnostics: &[Diagnostic]) -> ExitStatus {
    if diagnostics.iter().any(|d| d.code == Code::U8001) {
        ExitStatus::Internal
    } else if diagnostics.is_empty() {
        ExitStatus::Success
    } else {
        ExitStatus::Errors
    }
}

#[cfg(test)]
mod tests {
    use super::{Files, keeps_ceiling, reused};
    use una_diag::{Code, Diagnostic, Location, Position};

    fn files(lock: &str) -> Files {
        [
            ("src/m.una", "module m\n"),
            ("una.toml", "[package]\nname = \"c\"\n"),
            ("verification.lock", lock),
        ]
        .into_iter()
        .map(|(path, text)| (path.to_owned(), text.to_owned()))
        .collect()
    }

    fn found() -> Vec<Diagnostic> {
        let at = Position { line: 1, col: 1 };
        vec![Diagnostic::error(
            Code::U5001,
            "refuted".to_owned(),
            Location {
                file: "src/m.una".to_owned(),
                start: at,
                end: at,
            },
        )]
    }

    #[test]
    fn a_round_s_tier_one_is_reused_only_for_the_same_files() {
        let round = files("step_budget = 5000000\n");
        assert_eq!(
            reused(Some((round.clone(), found())), &round),
            Some(found())
        );
        // Only `verification.lock` changed since the round: the final check solves again.
        assert_eq!(
            reused(Some((round, found())), &files("step_budget = 1\n")),
            None
        );
        assert_eq!(reused(None, &files("step_budget = 1\n")), None);
    }

    #[test]
    fn a_boundary_edit_is_applied_only_as_one_narrowed_grant_of_u3020() {
        // ADR 0093 rule 6: the edit the checker writes within the ceiling.
        assert!(keeps_ceiling(Code::U3020, " with @files = @files[read]"));
        assert!(keeps_ceiling(Code::U3020, ", @net = @user_net[post, get]"));
        // A whole binding, a handler, a second binding, or no operation is never applied.
        for text in [
            " with @files = @files",
            " with @files = read_only()",
            " with @files = @files[read], @net = @net[post]",
            " with @files = @files[]",
            " with @files = @files[read] ",
            "with @files = @files[read]",
            "uses @files: Files[read, write]",
        ] {
            assert!(!keeps_ceiling(Code::U3020, text), "{text}");
        }
        // U3021 and U3024 never carry an applicable edit, whatever its text.
        assert!(!keeps_ceiling(Code::U3021, " with @files = @files[read]"));
        assert!(!keeps_ceiling(Code::U3024, " with @files = @files[read]"));
        // Other codes are not this rule's.
        assert!(keeps_ceiling(Code::U3002, " with @files = @store"));
    }
}
