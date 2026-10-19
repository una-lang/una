//! `una ita` (ADR 0122 rule 57), the package manager of edition `0.4`, and `una fmt` of its
//! project files (rule 5).
//!
//! This release resolves no dependency: `una ita lock` writes the lock of a package with no
//! `needs`, its solver section kept from the lock it replaces, or the default one; a need is
//! refused (U7008), never left unresolved. No command here opens a connection or starts a
//! program.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::json;
use una_diag::{Code, Diagnostic, render_human};
use una_project::ita::{
    check_lock_consistency, parse_ita_lock, parse_ita_manifest, write_lock,
};
use una_project::{Lock, ProjectError};
use una_syntax::{FileId, SourceDb, Span};

use crate::ExitStatus;

/// What `una ita` prints, and how.
pub(crate) struct Output<'a> {
    pub(crate) json: bool,
    pub(crate) color: bool,
    pub(crate) stdout: &'a mut dyn Write,
    pub(crate) stderr: &'a mut dyn Write,
}

/// The diagnostics of `errors`, located in the file `id` of `sources`.
fn located(sources: &SourceDb, id: FileId, errors: Vec<ProjectError>) -> Vec<Diagnostic> {
    errors
        .into_iter()
        .map(|error| {
            Diagnostic::error(
                error.code,
                error.message,
                sources.location(Span::new(id, error.range.start, error.range.end)),
            )
        })
        .collect()
}

/// Prints the result of a command: one `una.ita/1` document with `--json`, with `schema`
/// first; otherwise the diagnostics on stderr and `lines` on stdout. Exit code 1 when a
/// diagnostic is an error (`spec/13 §5`).
fn finish(
    command: &str,
    written: &[&str],
    lines: &[String],
    diagnostics: &[Diagnostic],
    sources: &SourceDb,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    let status = if diagnostics.is_empty() {
        ExitStatus::Success
    } else {
        ExitStatus::Errors
    };
    if out.json {
        let document = json!({
            "schema": "una.ita/1",
            "command": command,
            "written": written,
            "diagnostics": diagnostics,
        });
        writeln!(out.stdout, "{document}")?;
        out.stdout.flush()?;
    } else {
        write!(
            out.stderr,
            "{}",
            render_human(diagnostics, &sources.texts(), out.color)
        )?;
        for line in lines {
            writeln!(out.stdout, "{line}")?;
        }
    }
    Ok(status)
}

/// `una ita lock [--check] [path]` (ADR 0122 rules 14, 17 and 25).
///
/// Writes `ita.lock` for the package at `path` through a temporary file and a rename: its
/// solver section is the existing lock's, or the release's default when there is no lock yet.
/// An invalid `ita.una` or `ita.lock` is reported and nothing is written, so a pin is never
/// replaced silently. `--check` writes nothing, and reports U7009 where the lock is not the
/// one `una ita lock` would write. O(n) in the files' size.
pub(crate) fn lock_command(path: &Path, check: bool, out: Output<'_>) -> io::Result<ExitStatus> {
    let manifest_path = path.join("ita.una");
    let lock_path = path.join("ita.lock");
    if !manifest_path.is_file() {
        writeln!(
            out.stderr,
            "error: `{}` has no ita.una; `una ita` works on a package of edition 0.4, and `una migrate` moves a package to it",
            path.display()
        )?;
        return Ok(ExitStatus::Usage);
    }
    let mut sources = SourceDb::new();
    let manifest_text = match std::fs::read(&manifest_path) {
        Ok(bytes) => bytes,
        Err(error) => return unreadable(&manifest_path, &error, out),
    };
    let manifest_id = sources.add("ita.una", &manifest_text);
    let lock_id = if lock_path.is_file() {
        match std::fs::read(&lock_path) {
            Ok(bytes) => Some(sources.add("ita.lock", &bytes)),
            Err(error) => return unreadable(&lock_path, &error, out),
        }
    } else {
        None
    };
    let (manifest, errors) = parse_ita_manifest(sources.file(manifest_id).text());
    let mut diagnostics = located(&sources, manifest_id, errors);
    let needs = manifest.ita.as_ref().map_or(&[][..], |read| &read.needs[..]);
    diagnostics.extend(located(
        &sources,
        manifest_id,
        needs
            .iter()
            .map(|need| ProjectError {
                code: Code::U7008,
                range: need.range.clone(),
                message: format!(
                    "this release does not resolve dependencies yet, so `{}` cannot be locked",
                    need.name.value
                ),
            })
            .collect(),
    ));
    let existing = lock_id.map(|id| {
        let (lock, errors) = parse_ita_lock(sources.file(id).text());
        (id, lock, errors)
    });
    let solver = match &existing {
        Some((id, lock, errors)) => {
            diagnostics.extend(located(&sources, *id, errors.clone()));
            lock.solver.clone()
        }
        None => Lock::default(),
    };
    if !diagnostics.is_empty() {
        return finish("lock", &[], &[], &diagnostics, &sources, out);
    }
    let text = write_lock(&solver);
    if check {
        match &existing {
            None => diagnostics.extend(located(
                &sources,
                manifest_id,
                vec![missing_lock()],
            )),
            Some((id, lock, _)) => {
                if let Some(read) = manifest.ita.as_deref() {
                    let consistency = check_lock_consistency(read, lock);
                    diagnostics.extend(located(&sources, manifest_id, consistency.in_manifest));
                    diagnostics.extend(located(&sources, *id, consistency.in_lock));
                }
            }
        }
        return finish("lock", &[], &[], &diagnostics, &sources, out);
    }
    let unchanged = lock_id.is_some_and(|id| sources.file(id).text() == text);
    if unchanged {
        return finish(
            "lock",
            &[],
            &["ita.lock is up to date".to_owned()],
            &diagnostics,
            &sources,
            out,
        );
    }
    if let Err(error) = crate::write_through_rename_or_create(&lock_path, &text) {
        writeln!(
            out.stderr,
            "error: cannot write `{}`: {error}",
            lock_path.display()
        )?;
        return Ok(ExitStatus::Usage);
    }
    finish(
        "lock",
        &["ita.lock"],
        &["wrote ita.lock".to_owned()],
        &diagnostics,
        &sources,
        out,
    )
}

fn missing_lock() -> ProjectError {
    ProjectError {
        code: Code::U7009,
        range: 0..0,
        message: "the package has no ita.lock; run `una ita lock`".to_owned(),
    }
}

fn unreadable(path: &Path, error: &io::Error, out: Output<'_>) -> io::Result<ExitStatus> {
    writeln!(out.stderr, "error: cannot read `{}`: {error}", path.display())?;
    Ok(ExitStatus::Usage)
}

/// Whether `path` names a project file that `una fmt` formats when asked by name: `ita.una`
/// or `itaty.una` (ADR 0122 rule 14).
pub(crate) fn is_project_file(path: &Path) -> bool {
    path.is_file()
        && matches!(
            path.file_name().and_then(|name| name.to_str()),
            Some("ita.una" | "itaty.una")
        )
}

/// `una fmt ita.una` and `una fmt itaty.una` (ADR 0122 rule 5): rewrites the file in its
/// canonical form, or with `--check` reports U7001 where it differs. A file whose grammar it
/// does not meet is reported (U7008) and left untouched.
pub(crate) fn fmt_command(path: &Path, check: bool, out: Output<'_>) -> io::Result<ExitStatus> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => return unreadable(path, &error, out),
    };
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut sources = SourceDb::new();
    let id = sources.add(name.clone(), &bytes);
    let text = sources.file(id).text().to_owned();
    let syntax = if name == "itaty.una" {
        una_syntax::ita::parse_workspace(&text).1
    } else {
        una_syntax::ita::parse_manifest(&text).1
    };
    let errors: Vec<ProjectError> = syntax
        .into_iter()
        .map(|error| ProjectError {
            code: Code::U7008,
            range: error.range,
            message: error.message,
        })
        .collect();
    let mut diagnostics = located(&sources, id, errors);
    let canonical = if diagnostics.is_empty() {
        una_syntax::ita::format(&text)
    } else {
        None
    };
    let Some(canonical) = canonical else {
        return crate::report(
            &diagnostics,
            &sources,
            out.json,
            out.color,
            out.stdout,
            out.stderr,
        );
    };
    if check {
        diagnostics.extend(una_fmt::differences(&sources, id, &canonical));
    } else if text != canonical || sources.file(id).first_crlf().is_some() {
        let target: PathBuf = path.to_path_buf();
        if let Err(error) = crate::write_through_rename(&target, &canonical) {
            writeln!(out.stderr, "error: cannot write `{}`: {error}", target.display())?;
            return Ok(ExitStatus::Usage);
        }
    }
    crate::report(
        &diagnostics,
        &sources,
        out.json,
        out.color,
        out.stdout,
        out.stderr,
    )
}
