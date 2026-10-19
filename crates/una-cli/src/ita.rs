//! `una ita` (ADR 0122 rule 57), the package manager of edition `0.4`, and `una fmt` of its
//! project files (rule 5).
//!
//! This release resolves path dependencies only: `una ita lock` writes the solver section, kept
//! from the lock it replaces or the default one, and each path dependency; a need of a registry
//! or a workspace is refused (U7008), never left unresolved. No command here opens a connection
//! or starts a program.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use una_diag::{Code, Diagnostic, render_human};

use una_project::content;
use una_project::ita::{parse_ita_lock, parse_ita_manifest, recorded_ceiling, write_lock};
use una_project::migrate::text;
use una_project::version::Version;
use una_project::{Lock, ProjectError};
use una_syntax::ita;
use una_syntax::{FileId, SourceDb, Span};

use crate::ExitStatus;

/// `una.ita/1`, with `schema` first (ADR 0122 rule 57).
#[derive(Serialize)]
struct ItaDocument<'a> {
    schema: &'static str,
    command: &'a str,
    written: &'a [&'a str],
    diagnostics: &'a [Diagnostic],
}

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
        let document = ItaDocument {
            schema: "una.ita/1",
            command,
            written,
            diagnostics,
        };
        match serde_json::to_string(&document) {
            Ok(text) => writeln!(out.stdout, "{text}")?,
            Err(error) => {
                writeln!(out.stderr, "internal compiler error (U8001): {error}")?;
                return Ok(ExitStatus::Internal);
            }
        }
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

/// `una ita lock [--check] [--accept-authority] [path]` (ADR 0122 rules 14 to 17 and 25).
///
/// Writes `ita.lock` for the package at `path` through a temporary file and a rename: its
/// solver section is the existing lock's, or the release's default when there is none yet; then
/// each path dependency, with the version its `ita.una` gives, its directory, its content
/// digest, and the ceiling the root grants it. A ceiling wider than the one the lock records is
/// refused unless `--accept-authority` accepts it, so every widening is explicit and shows in
/// the lock's diff. An invalid `ita.una` or `ita.lock`, or a dependency the root may not use,
/// is reported and nothing is written. `--check` writes nothing and reports where the lock is
/// not the one this command writes (U7009). O(bytes of the dependencies), for their digests.
pub(crate) fn lock_command(
    path: &Path,
    check: bool,
    accept_authority: bool,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    let lock_path = path.join("ita.lock");
    if !path.join("ita.una").is_file() {
        writeln!(
            out.stderr,
            "error: `{}` has no ita.una; `una ita` works on a package of edition 0.4, and `una migrate` moves a package to it",
            path.display()
        )?;
        return Ok(ExitStatus::Usage);
    }
    let package = match crate::package::load(path) {
        Ok(package) => package,
        Err(error) => {
            writeln!(out.stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let sources = &package.sources;
    let Some(manifest_id) = package.manifest else {
        return Ok(ExitStatus::Usage);
    };
    let (manifest, errors) = parse_ita_manifest(sources.file(manifest_id).text());
    let mut diagnostics = located(sources, manifest_id, errors);
    let existing = package.lock.map(|id| {
        let (lock, errors) = parse_ita_lock(sources.file(id).text());
        (id, lock, errors)
    });
    let solver = match &existing {
        Some((id, lock, errors)) => {
            diagnostics.extend(located(sources, *id, errors.clone()));
            lock.solver.clone()
        }
        None => Lock::default(),
    };
    let Some(read) = manifest.ita.as_deref() else {
        return finish("lock", &[], &[], &diagnostics, sources, out);
    };
    let root = read
        .name
        .as_ref()
        .map(|name| name.value.to_string())
        .unwrap_or_default();
    let mut entries: Vec<(String, Version, String)> = Vec::new();
    for (index, need) in read.needs.iter().enumerate() {
        let refuse = |message: String| ProjectError {
            code: Code::U7008,
            range: need.range.clone(),
            message,
        };
        let Some(ita::At {
            value: ita::Origin::Path(_),
            ..
        }) = &need.origin
        else {
            diagnostics.extend(located(
                sources,
                manifest_id,
                vec![refuse(format!(
                    "this release resolves only path dependencies, so `{}` cannot be locked",
                    need.name.value
                ))],
            ));
            continue;
        };
        let Some(dependency) = package.dependencies.iter().find(|d| d.need == index) else {
            continue;
        };
        if let Some(reason) = &dependency.error {
            diagnostics.extend(located(
                sources,
                manifest_id,
                vec![refuse(format!(
                    "the path dependency `{}` cannot be loaded: {reason}",
                    dependency.dir
                ))],
            ));
            continue;
        }
        let Some(dependency_manifest) = dependency.manifest else {
            continue;
        };
        let (settings, errors) = parse_ita_manifest(sources.file(dependency_manifest).text());
        let invalid = !errors.is_empty();
        diagnostics.extend(located(sources, dependency_manifest, errors));
        let problems = crate::package::dependency_problems(need, &settings, &manifest);
        if invalid || !problems.is_empty() {
            diagnostics.extend(located(sources, manifest_id, problems));
            continue;
        }
        let Some(version) = settings
            .version
            .as_ref()
            .and_then(|v| Version::parse_full(&v.value).ok())
        else {
            continue;
        };
        // The digest of the files as they are on disk, read again in their bytes (rule 28).
        let mut files = Vec::new();
        for id in std::iter::once(dependency_manifest).chain(dependency.modules.iter().copied()) {
            let name = sources.file(id).path();
            let relative = name
                .strip_prefix(&format!("{}/", dependency.dir))
                .unwrap_or(name)
                .to_owned();
            let bytes = match package.disk_paths.get(&id).map(std::fs::read) {
                Some(Ok(bytes)) => bytes,
                _ => sources.file(id).text().as_bytes().to_vec(),
            };
            files.push(content::File {
                path: relative,
                bytes,
            });
        }
        let digest = match content::digest_of(&mut files) {
            Ok(digest) => digest,
            Err(reason) => {
                diagnostics.extend(located(
                    sources,
                    manifest_id,
                    vec![refuse(format!(
                        "the path dependency `{}` holds what a package may not: {reason}",
                        dependency.dir
                    ))],
                ));
                continue;
            }
        };
        let mut block = format!(
            "\npackage {} {version}\n  from path {}\n  digest \"{digest}\"\n",
            need.name.value,
            text(&dependency.dir)
        );
        let ceiling = canonical_ceiling(&need.may);
        if !ceiling.is_empty() {
            block.push_str(&format!("  granted by {root} may {ceiling}\n"));
        }
        // Consent (rule 25): a ceiling wider than the recorded one needs the flag.
        if !accept_authority
            && let Some((_, lock, _)) = &existing
        {
            let recorded = recorded_ceiling(lock, &need.name.value, &root);
            for cap in &need.may {
                let wider: Vec<&str> = cap
                    .ops
                    .iter()
                    .map(|op| op.value.as_str())
                    .filter(|op| {
                        !recorded
                            .get(cap.capability.as_str())
                            .is_some_and(|ops| ops.contains(op))
                    })
                    .collect();
                if !wider.is_empty() {
                    diagnostics.extend(located(
                        sources,
                        manifest_id,
                        vec![ProjectError {
                            code: Code::U7009,
                            range: cap.range.clone(),
                            message: format!(
                                "the ceiling of `{}` widens to `{}[{}]`; review what it grants and run `una ita lock --accept-authority`",
                                need.name.value,
                                cap.capability,
                                wider.join(", ")
                            ),
                        }],
                    ));
                }
            }
        } else if !accept_authority && !need.may.is_empty() {
            diagnostics.extend(located(
                sources,
                manifest_id,
                vec![ProjectError {
                    code: Code::U7009,
                    range: need.range.clone(),
                    message: format!(
                        "the ceiling of `{}` is not recorded yet; review what it grants and run `una ita lock --accept-authority`",
                        need.name.value
                    ),
                }],
            ));
        }
        entries.push((need.name.value.to_string(), version, block));
    }
    if !diagnostics.is_empty() {
        return finish("lock", &[], &[], &diagnostics, sources, out);
    }
    entries.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    let mut text = write_lock(&solver);
    for (_, _, block) in &entries {
        text.push_str(block);
    }
    if check {
        let same = existing
            .as_ref()
            .is_some_and(|(id, _, _)| sources.file(*id).text() == text);
        if !same {
            let error = match &existing {
                None => missing_lock(),
                Some(_) => ProjectError {
                    code: Code::U7009,
                    range: 0..0,
                    message: "ita.lock is not the lock of ita.una and its dependencies: a version, a digest or a ceiling changed; run `una ita lock`".to_owned(),
                },
            };
            let at = existing.as_ref().map_or(manifest_id, |(id, _, _)| *id);
            diagnostics.extend(located(sources, at, vec![error]));
        }
        return finish("lock", &[], &[], &diagnostics, sources, out);
    }
    let unchanged = existing
        .as_ref()
        .is_some_and(|(id, _, _)| sources.file(*id).text() == text);
    if unchanged {
        return finish(
            "lock",
            &[],
            &["ita.lock is up to date".to_owned()],
            &diagnostics,
            sources,
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
        sources,
        out,
    )
}

/// A ceiling in its canonical text: capabilities by name, each with its operations sorted,
/// as `Files[read], Http[get, post]`.
fn canonical_ceiling(caps: &[ita::CapType]) -> String {
    let mut by_name: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for cap in caps {
        by_name
            .entry(cap.capability.as_str())
            .or_default()
            .extend(cap.ops.iter().map(|op| op.value.as_str()));
    }
    by_name
        .iter()
        .map(|(name, ops)| {
            format!(
                "{name}[{}]",
                ops.iter().copied().collect::<Vec<_>>().join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn missing_lock() -> ProjectError {
    ProjectError {
        code: Code::U7009,
        range: 0..0,
        message: "the package has no ita.lock; run `una ita lock`".to_owned(),
    }
}

fn unreadable(path: &Path, error: &io::Error, out: Output<'_>) -> io::Result<ExitStatus> {
    writeln!(
        out.stderr,
        "error: cannot read `{}`: {error}",
        path.display()
    )?;
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
            writeln!(
                out.stderr,
                "error: cannot write `{}`: {error}",
                target.display()
            )?;
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
