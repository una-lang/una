//! `una migrate [path]` (ADR 0122 rule 64): moves a package from `una.toml` and
//! `verification.lock` to `ita.una` and `ita.lock` of edition `0.4`, or of edition `0.5` when
//! `una.toml` names it (no migration lowers an edition), and rewrites its modules in the
//! canonical form of edition `0.4`, which `0.5` keeps (ADR 0125, amendment of 2026-10-25).

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::Path;

use serde::Serialize;
use una_diag::{Diagnostic, render_human};
use una_project::Format;
use una_project::migrate::migrate;
use una_syntax::{FileId, Interner, KeywordSet, SourceDb};

use crate::ExitStatus;
use crate::package::{Package, load};

/// `una.migrate/1`, with `schema` first (ADR 0122 rule 64).
#[derive(Serialize)]
struct MigrateDocument<'a> {
    schema: &'static str,
    dry_run: bool,
    written: &'a [&'a str],
    removed: &'a [&'a str],
    name: NameDoc<'a>,
    edition: EditionDoc,
    default_lock: bool,
    comments_not_carried: Vec<CommentDoc<'a>>,
    ita_una: Option<&'a str>,
    ita_lock: Option<&'a str>,
    diagnostics: &'a [Diagnostic],
}

#[derive(Serialize)]
struct NameDoc<'a> {
    from: &'a str,
    to: &'a str,
}

#[derive(Serialize)]
struct EditionDoc {
    from: &'static str,
    to: &'static str,
}

#[derive(Serialize)]
struct CommentDoc<'a> {
    line: usize,
    text: &'a str,
}

/// The options of `una migrate`.
pub(crate) struct MigrateOptions<'a> {
    pub(crate) dry_run: bool,
    pub(crate) name: Option<&'a str>,
    pub(crate) json: bool,
    pub(crate) color: bool,
}

/// `una migrate [path] [--dry-run] [--name <name>]`.
///
/// Writes `ita.una` and `ita.lock` through temporary files and renames, rewrites each module
/// that parses in the edition it writes, `0.4` or the later `0.5` that `una.toml` named, in the
/// canonical form, which wraps long lines (ADR 0125, amendment of 2026-10-25; #1744), then
/// removes `una.toml` and `verification.lock`. Then checks the package, so rewritten, at tier 0
/// in that edition, and prints the diagnostics the change of edition causes: those it has in
/// that edition and not in its own. It applies none of their
/// repairs, which `una fix` does. With `--dry-run` it writes nothing. Edition-gated: a package that already has `ita.una`, or that is not a
/// package directory, is a usage error.
pub(crate) fn migrate_command(
    path: &Path,
    options: MigrateOptions<'_>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    if path.join("ita.una").exists() {
        writeln!(
            stderr,
            "error: `{}` already has ita.una; `una migrate` moves a package described by una.toml",
            path.display()
        )?;
        return Ok(ExitStatus::Usage);
    }
    let toml_path = path.join("una.toml");
    let lock_path = path.join("verification.lock");
    if !toml_path.is_file() || !path.join("src").is_dir() {
        writeln!(
            stderr,
            "error: `{}` is not a package with una.toml and src/",
            path.display()
        )?;
        return Ok(ExitStatus::Usage);
    }
    let old = match load(path) {
        Ok(package) => package,
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let toml_text = old
        .manifest
        .map(|id| old.sources.file(id).text().to_owned())
        .unwrap_or_default();
    let lock_text = old.lock.map(|id| old.sources.file(id).text().to_owned());
    let migration = match migrate(&toml_text, lock_text.as_deref(), options.name) {
        Ok(migration) => migration,
        Err(reasons) => {
            for reason in reasons {
                writeln!(stderr, "error: {reason}")?;
            }
            return Ok(ExitStatus::Usage);
        }
    };
    // The package as it will be, in memory, for the check of its new edition: its modules in
    // the canonical form of edition 0.4 (ADR 0125, amendment of 2026-10-25), read with the
    // keywords of the edition `ita.una` names, 0.4 or the later one `una.toml` named (#1744).
    let draft = migrated(&old, &migration.manifest, &migration.lock, &BTreeMap::new());
    let keywords = crate::keyword_set(migration.edition.max(una_project::Edition::V0_4));
    let canonical = canonical_form(&old, &draft, keywords);
    let new = migrated(&old, &migration.manifest, &migration.lock, &canonical);
    let before: BTreeSet<String> = crate::on_large_stack(|| crate::analyze(&old, None))
        .diagnostics
        .iter()
        .filter(|d| !is_project_file(&d.primary.file))
        .map(key)
        .collect();
    let caused: Vec<Diagnostic> = crate::on_large_stack(|| crate::analyze(&new, None))
        .diagnostics
        .into_iter()
        .filter(|d| !before.contains(&key(d)))
        .collect();
    let mut written: Vec<&str> = Vec::new();
    let mut removed: Vec<&str> = Vec::new();
    if !options.dry_run {
        for (name, text) in [
            ("ita.una", &migration.manifest),
            ("ita.lock", &migration.lock),
        ] {
            // `ita.lock` belongs to the toolchain: a link at its name is replaced, never
            // followed. `ita.una` is the author's, written as sources are.
            let target = path.join(name);
            let result = if name == "ita.lock" {
                una_diag::safe_write::write_new_name(path, &target, text.as_bytes(), true)
            } else {
                crate::write_through_rename_or_create(path, &target, text)
            };
            if let Err(error) = result {
                writeln!(stderr, "error: cannot write `{name}`: {error}")?;
                return Ok(ExitStatus::Usage);
            }
            written.push(name);
        }
        for (id, text) in &canonical {
            let Some(target) = old.disk_paths.get(id) else {
                continue;
            };
            if let Err(error) = crate::write_through_rename(path, target, text) {
                writeln!(
                    stderr,
                    "error: cannot write `{}`: {error}",
                    target.display()
                )?;
                return Ok(ExitStatus::Usage);
            }
            written.push(old.sources.file(*id).path());
        }
        for (name, file) in [("una.toml", &toml_path), ("verification.lock", &lock_path)] {
            if !file.exists() {
                continue;
            }
            if let Err(error) = std::fs::remove_file(file) {
                writeln!(stderr, "error: cannot remove `{name}`: {error}")?;
                return Ok(ExitStatus::Usage);
            }
            removed.push(name);
        }
    }
    let (from_name, to_name) = &migration.name;
    if options.json {
        let comments: Vec<CommentDoc<'_>> = migration
            .comments_left
            .iter()
            .map(|(line, text)| CommentDoc { line: *line, text })
            .collect();
        let document = MigrateDocument {
            schema: "una.migrate/1",
            dry_run: options.dry_run,
            written: &written,
            removed: &removed,
            name: NameDoc {
                from: from_name,
                to: to_name,
            },
            edition: EditionDoc {
                from: migration.edition.name(),
                to: migration.edition.max(una_project::Edition::V0_4).name(),
            },
            default_lock: migration.default_lock,
            comments_not_carried: comments,
            ita_una: options.dry_run.then_some(migration.manifest.as_str()),
            ita_lock: options.dry_run.then_some(migration.lock.as_str()),
            diagnostics: &caused,
        };
        let document = match serde_json::to_string(&document) {
            Ok(text) => text,
            Err(error) => {
                writeln!(stderr, "internal compiler error (U8001): {error}")?;
                return Ok(ExitStatus::Internal);
            }
        };
        writeln!(stdout, "{document}")?;
    } else {
        if options.dry_run {
            writeln!(stdout, "-- ita.una\n{}", migration.manifest)?;
            writeln!(stdout, "-- ita.lock\n{}", migration.lock)?;
        } else {
            writeln!(
                stdout,
                "wrote {}; removed {}",
                written.join(", "),
                if removed.is_empty() {
                    "nothing".to_owned()
                } else {
                    removed.join(", ")
                }
            )?;
        }
        if from_name != to_name {
            writeln!(stdout, "name: {from_name} -> {to_name}")?;
        }
        if migration.default_lock {
            writeln!(
                stdout,
                "the package had no verification.lock; ita.lock has the default solver section"
            )?;
        }
        for (line, text) in &migration.comments_left {
            writeln!(stdout, "not carried: una.toml:{line}: {text}")?;
        }
        let count = caused.len();
        writeln!(
            stdout,
            "edition {} -> {}: {count} diagnostic{}",
            migration.edition.name(),
            migration.edition.max(una_project::Edition::V0_4).name(),
            if count == 1 { "" } else { "s" }
        )?;
        write!(
            stderr,
            "{}",
            render_human(&caused, &new.sources.texts(), options.color)
        )?;
    }
    // The diagnostics are the next steps of a migration, not its failure.
    Ok(ExitStatus::Success)
}

/// The identity of a diagnostic across the two checks: code, file, position and message.
fn key(diagnostic: &Diagnostic) -> String {
    let at = &diagnostic.primary;
    format!(
        "{} {}:{}:{} {}",
        diagnostic.code, at.file, at.start.line, at.start.col, diagnostic.message
    )
}

fn is_project_file(file: &str) -> bool {
    matches!(file, "una.toml" | "verification.lock")
}

/// The modules of `old` whose text differs from their canonical form in the migrated edition,
/// read with `keywords`, its keyword set, by their ids in `old`, with that form, read from
/// `draft`, the package migrated as it is. The form is edition `0.4`'s, which edition `0.5`
/// keeps; a module with a syntax error under `keywords` has no canonical form and is left as it
/// is. One check of the package, as `una fmt` does: linear in its size.
fn canonical_form(
    old: &Package,
    draft: &Package,
    keywords: KeywordSet,
) -> BTreeMap<FileId, String> {
    let checks = crate::on_large_stack(|| {
        let mut interner = Interner::new();
        crate::check_files(
            &draft.sources,
            &crate::with_keywords(&draft.modules, keywords),
            &mut interner,
        )
    });
    // `migrated` adds the modules of `old` in order, so the ids correspond by position.
    let ids: BTreeMap<FileId, FileId> = draft
        .modules
        .iter()
        .copied()
        .zip(old.modules.iter().copied())
        .collect();
    checks
        .into_iter()
        .filter_map(|checked| {
            let canonical = checked.canonical?;
            let file = draft.sources.file(checked.id);
            if file.text() == canonical && file.first_crlf().is_none() {
                return None;
            }
            ids.get(&checked.id).map(|id| (*id, canonical))
        })
        .collect()
}

/// `old` with `ita.una` and `ita.lock` of the given texts in place of its project files, and
/// each module of `texts`, by its id in `old`, with that text.
fn migrated(
    old: &Package,
    manifest: &str,
    lock: &str,
    texts: &BTreeMap<FileId, String>,
) -> Package {
    let mut sources = SourceDb::new();
    let manifest_id = sources.add("ita.una", manifest.as_bytes());
    let lock_id = sources.add("ita.lock", lock.as_bytes());
    let mut disk_paths = std::collections::BTreeMap::new();
    let mut modules = Vec::new();
    for id in &old.modules {
        let file = old.sources.file(*id);
        let text = texts.get(id).map_or(file.text(), String::as_str);
        let new_id = sources.add(file.path().to_owned(), text.as_bytes());
        if let Some(path) = old.disk_paths.get(id) {
            disk_paths.insert(new_id, path.clone());
        }
        modules.push(new_id);
    }
    Package {
        sources,
        single: false,
        format: Format::Ita,
        both_manifests: false,
        manifest: Some(manifest_id),
        lock: Some(lock_id),
        lock_bytes: Some(lock.as_bytes().to_vec()),
        modules,
        disk_paths,
        enclosing_edition: None,
        dependencies: Vec::new(),
        registry: Vec::new(),
    }
}
