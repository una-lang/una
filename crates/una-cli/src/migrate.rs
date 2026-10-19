//! `una migrate [path]` (ADR 0122 rule 64): moves a package from `una.toml` and
//! `verification.lock` to `ita.una` and `ita.lock`, edition `0.4`.

use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::Path;

use serde::Serialize;
use una_diag::{Diagnostic, render_human};
use una_project::Format;
use una_project::migrate::migrate;
use una_syntax::SourceDb;

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
/// Writes `ita.una` and `ita.lock` through temporary files and renames, then removes
/// `una.toml` and `verification.lock`. Then checks the package at tier 0 in edition `0.4`, and
/// prints the diagnostics the change of edition causes: those it has in edition `0.4` and not
/// in its own. It applies none of their repairs, which `una fix` does. With `--dry-run` it
/// writes nothing. Edition-gated: a package that already has `ita.una`, or that is not a
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
    // The package as it will be, in memory, for the check of edition 0.4.
    let new = migrated(&old, &migration.manifest, &migration.lock);
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
        for (name, text) in [("ita.una", &migration.manifest), ("ita.lock", &migration.lock)] {
            if let Err(error) = crate::write_through_rename_or_create(&path.join(name), text) {
                writeln!(stderr, "error: cannot write `{name}`: {error}")?;
                return Ok(ExitStatus::Usage);
            }
            written.push(name);
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
                to: "0.4",
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
            "edition {} -> 0.4: {count} diagnostic{}",
            migration.edition.name(),
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

/// `old` with `ita.una` and `ita.lock` of the given texts in place of its project files.
fn migrated(old: &Package, manifest: &str, lock: &str) -> Package {
    let mut sources = SourceDb::new();
    let manifest_id = sources.add("ita.una", manifest.as_bytes());
    let lock_id = sources.add("ita.lock", lock.as_bytes());
    let mut disk_paths = std::collections::BTreeMap::new();
    let mut modules = Vec::new();
    for id in &old.modules {
        let file = old.sources.file(*id);
        let new_id = sources.add(file.path().to_owned(), file.text().as_bytes());
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
        modules,
        disk_paths,
    }
}
