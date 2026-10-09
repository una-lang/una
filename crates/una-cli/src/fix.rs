//! `una fix [path]`: applies the mechanical repairs that `una check` reports (ADR 0084).
//!
//! Edits are applied in memory, in rounds that each check the package again at tier 0, where
//! every mechanical edit comes from. Then every module that parses is formatted, each changed
//! file is written once, and the package is checked at the requested tier for what remains.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use una_diag::{Code, Diagnostic, Edit, Position, Repair, in_output_order};
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
    let stopped = loop {
        let Some(package) = reload(path, &modules, stderr)? else {
            return Ok(ExitStatus::Usage);
        };
        let diagnostics = on_large_stack(|| analyze(&package, None).diagnostics);
        let taken = take_round(&diagnostics, &package.sources, &modules);
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
        (a.round, &a.file, a.start.line, a.start.col).cmp(&(b.round, &b.file, b.start.line, b.start.col))
    });
    let Some(formatted) = format_modules(path, &mut modules, stderr)? else {
        return Ok(ExitStatus::Usage);
    };
    if !options.dry_run {
        for (name, module) in &modules {
            if module.invalid || original.get(name) == Some(&module.text) && !module.crlf {
                continue;
            }
            if let Err(error) = crate::write_through_rename(&module.disk, &module.text) {
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
    let skipped = skipped_repairs(&diagnostics, &modules, stopped);
    if options.tier == Tier::One && diagnostics.is_empty() {
        diagnostics = verification::tier_one(&package, &program, &options.solving).diagnostics;
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
            let invalid = file
                .text()
                .char_indices()
                .any(|(offset, c)| c == char::REPLACEMENT_CHARACTER && file.is_invalid_sequence(offset));
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

/// The first repair of `diagnostic`, in the order `spec/13 §1` prints them: by escape cost,
/// then rank.
fn first_repair(diagnostic: &Diagnostic) -> Option<&Repair> {
    diagnostic
        .repairs
        .iter()
        .min_by_key(|repair| (repair.escape_cost, repair.rank))
}

/// Why a first repair with an edit is not applied; `None` when it is a candidate.
fn ineligible(
    diagnostic: &Diagnostic,
    repair: &Repair,
    edit: &Edit,
    modules: &BTreeMap<String, Module>,
) -> Option<&'static str> {
    if repair.escape_cost > 0 {
        return Some("escape cost");
    }
    if repair.widens_authority {
        return Some("widens authority");
    }
    match modules.get(&edit.location.file) {
        None => Some("not a module"),
        Some(module) if module.invalid => Some("invalid UTF-8"),
        // The final `una fmt` step formats every file that parses.
        Some(_) if diagnostic.code == Code::U7001 => Some("formatting"),
        Some(_) => None,
    }
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
    for (index, diagnostic) in in_output_order(diagnostics).into_iter().enumerate() {
        let Some(repair) = first_repair(diagnostic) else {
            continue;
        };
        let Some(edit) = &repair.edit else {
            continue;
        };
        if ineligible(diagnostic, repair, edit, modules).is_some() {
            continue;
        }
        let Some((start, end)) = range(sources, edit) else {
            continue;
        };
        by_file.entry(edit.location.file.clone()).or_default().push((
            (start, end, edit.clone(), diagnostic.code, repair.description.clone()),
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
        check_files(&package.sources, &package.modules, &mut interner, keywords)
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
    in_output_order(diagnostics)
        .into_iter()
        .filter_map(|diagnostic| {
            let repair = first_repair(diagnostic)?;
            let edit = repair.edit.as_ref()?;
            let reason = match ineligible(diagnostic, repair, edit, modules) {
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
