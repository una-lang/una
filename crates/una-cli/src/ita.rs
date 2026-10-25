//! `una ita` (ADR 0122 rule 57), the package manager of edition `0.4`, and `una fmt` of its
//! project files (rule 5).
//!
//! This release resolves path dependencies only: `una ita lock`, `update` and `add` write the
//! solver section, kept from the lock it replaces or the default one, and each path dependency,
//! resolved by minimal version selection over what is held offline. A need of a registry is
//! refused (U7013), since no registry's per-name state is held, and one of a workspace member is
//! refused (U7008): never left unresolved, and never resolved from a partial view. No command
//! here opens a connection or starts a program.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use una_diag::{Code, Diagnostic, render_human};

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

/// What `una ita lock`, `update`, `add` and `remove` ask of a resolution (ADR 0122 rule 25).
pub(crate) struct LockOptions<'a> {
    /// The command, as `una.ita/1` names it.
    pub(crate) command: &'static str,
    /// `--check`: write nothing, and report where the lock is not the one written (U7009).
    pub(crate) check: bool,
    /// `--dry-run`: print the review, and write nothing.
    pub(crate) dry_run: bool,
    /// `--accept-authority <package>`, repeated: the packages whose ceiling may be recorded wider
    /// than the lock records it.
    pub(crate) accept_authority: &'a [String],
    /// `--accept-trust <package>@sha256:<hex>`, repeated: consent to the trust of a package for
    /// exactly the content of that digest.
    pub(crate) accept_trust: &'a [String],
    /// The cooldown of rule 25, in seconds: 72 hours by default, 0 with `--now`.
    pub(crate) cooldown: u64,
    /// `una ita update <name>…`: the needs to raise. Every other need that the lock records
    /// keeps what it records. `None`: every need.
    pub(crate) only: Option<&'a [String]>,
    /// `ita.una` as `una ita add` or `remove` edits it: read in place of the file, and written,
    /// after the lock, only when the resolution succeeds.
    pub(crate) edited: Option<String>,
}

impl<'a> LockOptions<'a> {
    /// The options of `una ita lock`.
    pub(crate) fn lock(
        check: bool,
        dry_run: bool,
        accept_authority: &'a [String],
        accept_trust: &'a [String],
    ) -> LockOptions<'a> {
        LockOptions {
            command: "lock",
            check,
            dry_run,
            accept_authority,
            accept_trust,
            cooldown: una_project::resolve::COOLDOWN,
            only: None,
            edited: None,
        }
    }
}

/// One change of the lock, for the review that every change prints before it is written
/// (ADR 0122 rule 25).
#[derive(Clone, Debug, Serialize)]
struct ChangeDoc {
    /// `add`, `change` or `drop`.
    change: &'static str,
    package: String,
    /// The origin, as the lock writes it: `path deps/util`.
    origin: String,
    version_from: Option<String>,
    version_to: Option<String>,
    digest_from: Option<String>,
    digest_to: Option<String>,
    /// The ceiling the root grants it, before and after.
    ceiling_from: String,
    ceiling_to: String,
    /// Its trusted modules the lock records consent to, before and after.
    trusted_from: Vec<String>,
    trusted_to: Vec<String>,
    /// The content digest that consent is pinned to, before and after.
    consented_from: Option<String>,
    consented_to: Option<String>,
    /// What its code uses in the new graph, checked at tier 0: the union of the `uses` of its
    /// functions and handlers.
    authority: Vec<String>,
}

impl ChangeDoc {
    /// The review line: `add util 0.2.0 (path deps/util): digest sha256:...; may Console[print];
    /// uses Console[print]`, then `; trust keys; consented sha256:...` for a package with
    /// trusted modules before or after.
    fn line(&self) -> String {
        let version = match (&self.version_from, &self.version_to) {
            (Some(from), Some(to)) if from != to => format!("{from} -> {to}"),
            (_, Some(to)) => to.clone(),
            (Some(from), None) => from.clone(),
            (None, None) => String::new(),
        };
        let mut line = format!(
            "{} {} {version} ({})",
            self.change, self.package, self.origin
        );
        if self.change == "drop" {
            return line;
        }
        let digest = match (&self.digest_from, &self.digest_to) {
            (Some(from), Some(to)) if from != to => format!("{from} -> {to}"),
            (_, Some(to)) => to.clone(),
            _ => String::new(),
        };
        let none = |text: &str| {
            if text.is_empty() {
                "nothing".to_owned()
            } else {
                text.to_owned()
            }
        };
        let ceiling = if self.change == "change" && self.ceiling_from != self.ceiling_to {
            format!("{} -> {}", none(&self.ceiling_from), none(&self.ceiling_to))
        } else {
            none(&self.ceiling_to)
        };
        let uses = if self.authority.is_empty() {
            "nothing".to_owned()
        } else {
            self.authority.join(", ")
        };
        line.push_str(&format!(": digest {digest}; may {ceiling}; uses {uses}"));
        if !self.trusted_from.is_empty() || !self.trusted_to.is_empty() {
            let modules = |list: &[String]| {
                if list.is_empty() {
                    "none".to_owned()
                } else {
                    list.join(", ")
                }
            };
            let trust = if self.change == "change" && self.trusted_from != self.trusted_to {
                format!(
                    "{} -> {}",
                    modules(&self.trusted_from),
                    modules(&self.trusted_to)
                )
            } else {
                modules(&self.trusted_to)
            };
            let consented = match (&self.consented_from, &self.consented_to) {
                (Some(from), Some(to)) if from != to => format!("{from} -> {to}"),
                (None, Some(to)) if self.change == "change" => format!("none -> {to}"),
                (_, Some(to)) => to.clone(),
                (Some(from), None) => format!("{from} -> none"),
                (None, None) => "none".to_owned(),
            };
            line.push_str(&format!("; trust {trust}; consented {consented}"));
        }
        line
    }
}

/// `una.ita/1` of a command that resolves: [`ItaDocument`] with the review's changes.
#[derive(Serialize)]
struct ResolveDocument<'a> {
    schema: &'static str,
    command: &'a str,
    written: &'a [&'a str],
    changes: &'a [ChangeDoc],
    diagnostics: &'a [Diagnostic],
}

/// A path dependency as the resolution locks it.
struct Locked {
    name: String,
    version: Version,
    dir: String,
    digest: String,
    ceiling: String,
    /// The trusted modules consented to, sorted, and the digest the consent is pinned to.
    trusted: Vec<String>,
    consented: Option<String>,
}

impl Locked {
    /// Its block of `ita.lock`, granted by `root`.
    fn block(&self, root: &str) -> String {
        let mut block = format!(
            "\npackage {} {}\n  from path {}\n  digest \"{}\"\n",
            self.name,
            self.version,
            text(&self.dir),
            self.digest
        );
        if !self.ceiling.is_empty() {
            block.push_str(&format!("  granted by {root} may {}\n", self.ceiling));
        }
        if let Some(consented) = &self.consented {
            block.push_str(&format!(
                "  consented \"{consented}\"\n  trusted modules {}\n",
                self.trusted.join(", ")
            ));
        }
        block
    }
}

/// The locked package of `lock` that `need`, a path need of the directory `dir`, names: by name
/// and origin (ADR 0122 rule 25).
fn locked_path<'l>(
    lock: &'l una_project::ita::ItaLock,
    need: &ita::Need,
    dir: &str,
) -> Option<&'l ita::LockPackage> {
    lock.read.packages.iter().find(|p| {
        p.name.value == need.name.value
            && matches!(
                p.from.as_ref().map(|f| &f.value),
                Some(ita::LockOrigin::Path(locked)) if locked.trim_end_matches('/') == dir
            )
    })
}

/// What `lock` records for the path need `need` of `dir`, kept as it is by `una ita update`
/// for a need it is not asked to raise: its version, digest, the ceiling `root` grants it, and
/// its consented trust. `None` when the lock records no complete entry for it.
fn kept_entry(
    lock: &una_project::ita::ItaLock,
    need: &ita::Need,
    dir: &str,
    root: &str,
) -> Option<Locked> {
    let package = locked_path(lock, need, dir)?;
    let version = Version::parse_full(&package.version.value).ok()?;
    let digest = package.digest.as_ref()?.value.clone();
    let ceiling = package
        .granted
        .iter()
        .find(|g| g.identity.is_none() && g.by.value.to_string() == root)
        .map(|g| canonical_ceiling(&g.may))
        .unwrap_or_default();
    let mut trusted: Vec<String> = package.trusted.iter().map(|m| m.value.clone()).collect();
    trusted.sort();
    Some(Locked {
        name: need.name.value.to_string(),
        version,
        dir: dir.to_owned(),
        digest,
        ceiling,
        trusted,
        consented: package.consented.as_ref().map(|c| c.value.clone()),
    })
}

/// A digest as `una ita` writes it: `sha256:` and 64 lowercase hexadecimal digits.
fn is_digest(text: &str) -> bool {
    text.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// `--accept-trust <package>@sha256:<hex>`: the package and the digest, when it has that form.
fn trust_consent(flag: &str) -> Option<(&str, &str)> {
    let (name, digest) = flag.rsplit_once('@')?;
    (!name.is_empty() && is_digest(digest)).then_some((name, digest))
}

/// `una ita lock [--check] [--dry-run] [--accept-authority <p>] [--accept-trust <p>@<digest>]
/// [path]` (ADR 0122 rules 14 to 17, 23 and 25).
pub(crate) fn lock_command(
    path: &Path,
    options: LockOptions<'_>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    resolve_and_lock(path, options, out)
}

/// The internal error of a need the resolution would otherwise drop: an invariant that the
/// checks before it hold, reported (U8001) rather than leaving the need out of the lock.
fn dropped(range: std::ops::Range<usize>, what: String) -> ProjectError {
    ProjectError {
        code: Code::U8001,
        range,
        message: format!("internal error: {what}; please report it"),
    }
}

/// Resolves the package at `path` and writes `ita.lock` through a temporary file and a rename.
///
/// Resolution is minimal version selection (rules 23 and 24) over the releases held offline:
/// each path dependency as its directory holds it, one release of its identity. A need of a
/// registry cannot be resolved offline in this release, since the per-name state of a
/// resolution point is not held (U7013): no registry is resolved against a partial view. A
/// workspace member is not loaded (U7008). An avoided version that no later release replaces,
/// and a bound of rule 23 exceeded, are U7013. `una ita update <name>…` resolves only the named
/// needs again; each other need the lock records keeps its entry as recorded.
///
/// The lock's solver section is the existing lock's, or the release's default; then each path
/// dependency, with its version, directory, content digest, and the ceiling the root grants
/// it. Consent (rule 25), each flag naming what it accepts: a ceiling wider than the one the
/// lock records needs `--accept-authority <package>`, and a trust that no consent covers for
/// the package's content as it is now needs `--accept-trust <package>@<digest>`, the digest of
/// that content. Every change is printed as a review before it is written, and when consent is
/// missing too, with the flags to accept it, and nothing is written: per package, its origin,
/// old and new version and digest, its ceiling, its consented trust, and what its code uses in
/// the new graph, checked at tier 0. `--dry-run` prints the review and writes nothing; `--check`
/// writes nothing and reports where the lock is not the one this command writes (U7009).
/// `ita.una`, when `add` or `remove` edits it, is written after the lock, and the lock is put
/// back when it cannot be. Costs: O(bytes of the dependencies) for their digests, and
/// O((n + p) log p) for n needs and p packages to resolve and sort.
fn resolve_and_lock(
    path: &Path,
    options: LockOptions<'_>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    use una_project::resolve::{self, Bounds, Identity, Need, Offline, Policy, Problem, Release};

    let command = options.command;
    // The command whose `--accept-…` flags repair a refusal: `update` takes them, and `lock`
    // repairs one of `add` or `remove`, which do not.
    let repair = if command == "update" { command } else { "lock" };
    let lock_path = path.join("ita.lock");
    let manifest_path = path.join("ita.una");
    if !manifest_path.is_file() {
        writeln!(
            out.stderr,
            "error: `{}` has no ita.una; `una ita` works on a package of edition 0.4, and `una migrate` moves a package to it",
            path.display()
        )?;
        return Ok(ExitStatus::Usage);
    }
    for name in options.accept_authority {
        if name.is_empty() || name.starts_with('-') {
            writeln!(
                out.stderr,
                "error: `--accept-authority` names the package whose wider ceiling it accepts, as `--accept-authority util`, which the review prints"
            )?;
            return Ok(ExitStatus::Usage);
        }
    }
    let mut trust_consents: Vec<(&str, &str)> = Vec::new();
    for flag in options.accept_trust {
        let Some(consent) = trust_consent(flag) else {
            writeln!(
                out.stderr,
                "error: `--accept-trust {flag}` does not name a package and a digest; write `--accept-trust <package>@sha256:<hex>` with the digest of the content reviewed, which the review prints"
            )?;
            return Ok(ExitStatus::Usage);
        };
        trust_consents.push(consent);
    }
    let mut overlay: BTreeMap<PathBuf, String> = BTreeMap::new();
    if let Some(edited) = &options.edited {
        overlay.insert(manifest_path.clone(), edited.clone());
    }
    let package = match crate::package::load_with(path, &overlay) {
        Ok(package) => package,
        Err(error) => {
            writeln!(out.stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let sources = &package.sources;
    let Some(manifest_id) = package.manifest else {
        writeln!(
            out.stderr,
            "internal compiler error (U8001): `{}` was loaded without its ita.una; please report it",
            path.display()
        )?;
        return Ok(ExitStatus::Internal);
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
        return finish_resolved(command, &[], &[], &[], &diagnostics, sources, out);
    };
    let root = read
        .name
        .as_ref()
        .map(|name| name.value.to_string())
        .unwrap_or_default();
    // The releases held offline, the root's needs on them, and the avoided versions.
    let mut index = Offline::default();
    let mut root_needs: Vec<Need> = Vec::new();
    let mut need_ranges: Vec<(Identity, std::ops::Range<usize>)> = Vec::new();
    let mut policy = Policy {
        now: 0,
        cooldown: options.cooldown,
        avoid: BTreeMap::new(),
    };
    let mut entries: Vec<Locked> = Vec::new();
    // Consent missing (rule 25): reported after the review, with the flags that accept it.
    let mut consent: Vec<ProjectError> = Vec::new();
    let mut accept: Vec<String> = Vec::new();
    for (index_of_need, need) in read.needs.iter().enumerate() {
        let refuse = |code: Code, message: String| ProjectError {
            code,
            range: need.range.clone(),
            message,
        };
        let name = need.name.value.to_string();
        let source = match need.origin.as_ref().map(|origin| &origin.value) {
            Some(ita::Origin::Path(_)) => None,
            Some(ita::Origin::Workspace) => {
                diagnostics.extend(located(
                    sources,
                    manifest_id,
                    vec![refuse(
                        Code::U7008,
                        format!(
                            "this release does not load workspace members, so `{name}` cannot be locked"
                        ),
                    )],
                ));
                continue;
            }
            Some(ita::Origin::Source(source)) => Some(source.clone()),
            None => Some(una_project::ita::ITATY.to_owned()),
        };
        if let Some(source) = source {
            // Rule 23 reads a registry's releases from its per-name state at a resolution
            // point, which this release does not hold: refused, never resolved from a part.
            let reason = format!(
                "`{name}` cannot be resolved offline, since the per-name state of `{source}` at a resolution point is not held; this release resolves path dependencies only"
            );
            index.refuse(&source, reason.clone());
            diagnostics.extend(located(
                sources,
                manifest_id,
                vec![refuse(Code::U7013, reason)],
            ));
            continue;
        }
        let Some(dependency) = package
            .dependencies
            .iter()
            .find(|d| d.need == index_of_need)
        else {
            diagnostics.extend(located(
                sources,
                manifest_id,
                vec![dropped(
                    need.range.clone(),
                    format!("the path dependency of `{name}` was not loaded"),
                )],
            ));
            continue;
        };
        if let Some(reason) = &dependency.error {
            diagnostics.extend(located(
                sources,
                manifest_id,
                vec![refuse(
                    Code::U7008,
                    format!(
                        "the path dependency `{}` cannot be loaded: {reason}",
                        dependency.dir
                    ),
                )],
            ));
            continue;
        }
        let Some(dependency_manifest) = dependency.manifest else {
            diagnostics.extend(located(
                sources,
                manifest_id,
                vec![dropped(
                    need.range.clone(),
                    format!(
                        "the path dependency `{}` was loaded without its ita.una",
                        dependency.dir
                    ),
                )],
            ));
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
        // `una ita update <name>…`: a need it is not asked to raise keeps what the lock
        // records, its consents included.
        let kept = options
            .only
            .filter(|only| !only.contains(&name))
            .and_then(|_| existing.as_ref())
            .and_then(|(_, lock, _)| kept_entry(lock, need, &dependency.dir, &root));
        let entry = if let Some(kept) = kept {
            kept
        } else {
            // `version` is required, and a full version, or `ita.una` reported an error above.
            let Some(version) = settings
                .version
                .as_ref()
                .and_then(|v| Version::parse_full(&v.value).ok())
            else {
                diagnostics.extend(located(
                    sources,
                    manifest_id,
                    vec![dropped(
                        need.range.clone(),
                        format!(
                            "the path dependency `{}` has no version, though its ita.una read without an error",
                            dependency.dir
                        ),
                    )],
                ));
                continue;
            };
            // The digest of exactly the bytes loaded, which the checks read (rule 28).
            let digest = match crate::package::dependency_digest(dependency) {
                Ok(digest) => digest,
                Err(reason) => {
                    diagnostics.extend(located(
                        sources,
                        manifest_id,
                        vec![refuse(
                            Code::U7008,
                            format!(
                                "the path dependency `{}` holds what a package may not: {reason}",
                                dependency.dir
                            ),
                        )],
                    ));
                    continue;
                }
            };
            let ceiling = canonical_ceiling(&need.may);
            // Consent (rule 25): a ceiling wider than the recorded one needs the flag that
            // names this package.
            if !options.accept_authority.contains(&name) {
                let flag = format!("--accept-authority {name}");
                if let Some((_, lock, _)) = &existing {
                    let origin = need.origin.as_ref().map(|origin| &origin.value);
                    let recorded = recorded_ceiling(lock, &need.name.value, origin, &root);
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
                            consent.push(ProjectError {
                                code: Code::U7009,
                                range: cap.range.clone(),
                                message: format!(
                                    "the ceiling of `{name}` widens to `{}[{}]`; review what it grants and run `una ita {repair} {flag}`",
                                    cap.capability,
                                    wider.join(", ")
                                ),
                            });
                            if !accept.contains(&flag) {
                                accept.push(flag.clone());
                            }
                        }
                    }
                } else if !need.may.is_empty() {
                    consent.push(ProjectError {
                        code: Code::U7009,
                        range: need.range.clone(),
                        message: format!(
                            "the ceiling of `{name}` is not recorded yet; review what it grants and run `una ita {repair} {flag}`"
                        ),
                    });
                    accept.push(flag);
                }
            }
            // Trust (rules 10, 25 and 53): consent to a dependency's trusted modules is pinned
            // to its whole content, and recorded only by a flag that names that content.
            let mut trusted: Vec<String> = Vec::new();
            let mut consented = None;
            if !settings.trusted.is_empty() {
                trusted = settings.trusted.iter().map(|t| t.value.clone()).collect();
                trusted.sort_unstable();
                let line = crate::package::trust_line(
                    read,
                    need,
                    settings.version.as_ref().map(|v| v.value.as_str()),
                );
                let covered = line.is_some_and(|line| {
                    trusted
                        .iter()
                        .all(|t| line.modules.iter().any(|m| m.value == *t))
                });
                if !covered {
                    diagnostics.extend(located(
                        sources,
                        manifest_id,
                        vec![ProjectError {
                            code: Code::U7012,
                            range: need.range.clone(),
                            message: format!(
                                "`{name}` has trusted modules, which no trust line of the root names; write `trust` with `{name} modules {}` after reviewing them",
                                trusted.join(", ")
                            ),
                        }],
                    ));
                    continue;
                }
                let recorded = existing.as_ref().is_some_and(|(_, lock, _)| {
                    locked_path(lock, need, &dependency.dir).is_some_and(|p| {
                        p.consented.as_ref().is_some_and(|c| c.value == digest)
                            && trusted
                                .iter()
                                .all(|t| p.trusted.iter().any(|m| m.value == *t))
                    })
                });
                let accepted = trust_consents
                    .iter()
                    .any(|(consented, at)| *consented == name && *at == digest);
                if !recorded && !accepted {
                    let flag = format!("--accept-trust {name}@{digest}");
                    let stale = trust_consents
                        .iter()
                        .find(|(consented, _)| *consented == name)
                        .map(|(_, at)| *at);
                    let message = match stale {
                        Some(at) => format!(
                            "`--accept-trust {name}@{at}` consents to other content than `{name}` holds now, {digest}; review its trusted modules and the code they run again, then run `una ita {repair} {flag}`"
                        ),
                        None => format!(
                            "the trust of `{name}` is not consented for its content as it is now, {digest}; review its trusted modules and the code they run, then run `una ita {repair} {flag}`"
                        ),
                    };
                    consent.push(ProjectError {
                        code: Code::U7012,
                        range: need.range.clone(),
                        message,
                    });
                    accept.push(flag);
                }
                consented = Some(digest.clone());
            }
            Locked {
                name: name.clone(),
                version,
                dir: dependency.dir.clone(),
                digest,
                ceiling,
                trusted,
                consented,
            }
        };
        // One release of the identity, as the directory holds it or the lock records it; its
        // own needs are refused by `dependency_problems` in this release, so it has none.
        let identity = Identity {
            source: format!("path {}", dependency.dir),
            name: name.clone(),
            line: entry.version.line(),
        };
        index.add(
            identity.clone(),
            Release {
                version: entry.version.clone(),
                yanked: false,
                advised: false,
                threshold_fix: false,
                logged_at: 0,
            },
            Vec::new(),
        );
        for avoid in &need.avoid {
            if let Ok(avoided) = Version::parse_full(&avoid.value) {
                policy
                    .avoid
                    .entry(identity.clone())
                    .or_default()
                    .insert(avoided);
            }
        }
        root_needs.push(Need {
            source: identity.source.clone(),
            name: identity.name.clone(),
            version: entry.version.clone(),
        });
        need_ranges.push((identity, need.range.clone()));
        entries.push(entry);
    }
    if !diagnostics.is_empty() {
        diagnostics.extend(located(sources, manifest_id, consent));
        return finish_resolved(command, &[], &[], &[], &diagnostics, sources, out);
    }
    // Minimal version selection over what is held (rules 23 and 24), within its bounds.
    let resolution = match resolve::resolve(&root_needs, &index, &policy, Bounds::default()) {
        Ok(resolution) => resolution,
        Err(problem) => {
            let at = match &problem {
                Problem::NoRelease { identity, .. }
                | Problem::NoRaise { identity, .. }
                | Problem::PreRelease { identity, .. }
                | Problem::SelfNeed(identity) => need_ranges
                    .iter()
                    .find(|(id, _)| id == identity)
                    .map(|(_, range)| range.clone()),
                Problem::Bound(_) | Problem::Index(_) => None,
            };
            let error = ProjectError {
                code: Code::U7013,
                range: at.unwrap_or(0..0),
                message: problem.to_string(),
            };
            diagnostics.extend(located(sources, manifest_id, vec![error]));
            return finish_resolved(command, &[], &[], &[], &diagnostics, sources, out);
        }
    };
    // Every root need is in the build list, at the version its directory holds: a path
    // package has one release. One that is not would be dropped from the lock: an internal
    // error, never a lock without it.
    let selected: BTreeSet<(String, String, String)> = resolution
        .list
        .iter()
        .map(|(identity, version)| {
            (
                identity.name.clone(),
                identity.source.clone(),
                version.to_string(),
            )
        })
        .collect();
    let missing: Vec<ProjectError> = entries
        .iter()
        .zip(&need_ranges)
        .filter(|(entry, _)| {
            !selected.contains(&(
                entry.name.clone(),
                format!("path {}", entry.dir),
                entry.version.to_string(),
            ))
        })
        .map(|(entry, (_, range))| {
            dropped(
                range.clone(),
                format!(
                    "`{} {}` is not in the build list of its own need",
                    entry.name, entry.version
                ),
            )
        })
        .collect();
    if !missing.is_empty() {
        diagnostics.extend(located(sources, manifest_id, missing));
        return finish_resolved(command, &[], &[], &[], &diagnostics, sources, out);
    }
    entries.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    let mut text = write_lock(&solver);
    for entry in &entries {
        text.push_str(&entry.block(&root));
    }
    let lock_unchanged = existing
        .as_ref()
        .is_some_and(|(id, _, _)| sources.file(*id).text() == text);
    if options.check {
        if !lock_unchanged {
            let error = match &existing {
                None => missing_lock(),
                Some(_) => ProjectError {
                    code: Code::U7009,
                    range: 0..0,
                    message: "ita.lock is not the lock of ita.una and its dependencies: a version, a digest, a ceiling or a consent changed; run `una ita lock`".to_owned(),
                },
            };
            let at = existing.as_ref().map_or(manifest_id, |(id, _, _)| *id);
            diagnostics.extend(located(sources, at, vec![error]));
        }
        diagnostics.extend(located(sources, manifest_id, consent));
        return finish_resolved(command, &[], &[], &[], &diagnostics, sources, out);
    }
    if options.edited.is_none() && lock_unchanged && consent.is_empty() {
        return finish_resolved(
            command,
            &[],
            &[],
            &["ita.lock is up to date".to_owned()],
            &diagnostics,
            sources,
            out,
        );
    }
    let changes = review(
        &package,
        existing.as_ref().map(|(_, lock, _)| lock),
        &entries,
    );
    let mut lines: Vec<String> = changes.iter().map(ChangeDoc::line).collect();
    if !consent.is_empty() {
        // The review is printed, then the flags that accept what it shows: nothing is written.
        lines.push(format!(
            "to accept, after reviewing: una ita {repair} {}",
            accept.join(" ")
        ));
        lines.push("nothing written".to_owned());
        diagnostics.extend(located(sources, manifest_id, consent));
        return finish_resolved(command, &[], &changes, &lines, &diagnostics, sources, out);
    }
    if options.dry_run {
        lines.push("--dry-run: nothing written".to_owned());
        return finish_resolved(command, &[], &changes, &lines, &diagnostics, sources, out);
    }
    let mut written: Vec<&str> = Vec::new();
    // The lock as it was, to put back if `ita.una` cannot be written after it.
    let previous: Option<Vec<u8>> = if options.edited.is_some() && !lock_unchanged {
        match std::fs::read(&lock_path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                writeln!(
                    out.stderr,
                    "error: cannot read `{}` to keep it: {error}; nothing was written",
                    lock_path.display()
                )?;
                return Ok(ExitStatus::Errors);
            }
        }
    } else {
        None
    };
    if !lock_unchanged {
        if let Err(error) = crate::write_through_rename_or_create(&lock_path, &text) {
            writeln!(
                out.stderr,
                "error: cannot write `{}`: {error}; nothing was written",
                lock_path.display()
            )?;
            return Ok(ExitStatus::Errors);
        }
        written.push("ita.lock");
        lines.push("wrote ita.lock".to_owned());
    }
    if let Some(edited) = &options.edited {
        if let Err(error) = crate::write_through_rename(&manifest_path, edited) {
            writeln!(
                out.stderr,
                "error: cannot write `{}`: {error}",
                manifest_path.display()
            )?;
            if !lock_unchanged {
                let restored = match &previous {
                    Some(bytes) => crate::write_through_rename_or_create(
                        &lock_path,
                        &String::from_utf8_lossy(bytes),
                    ),
                    None => std::fs::remove_file(&lock_path),
                };
                match restored {
                    Ok(()) => writeln!(
                        out.stderr,
                        "error: `{}` was put back as it was; nothing was changed",
                        lock_path.display()
                    )?,
                    Err(error) => writeln!(
                        out.stderr,
                        "error: `{}` could not be put back as it was: {error}; it is the lock of the edit that was not written, so run `una ita lock`",
                        lock_path.display()
                    )?,
                }
            }
            return Ok(ExitStatus::Errors);
        }
        written.push("ita.una");
        lines.push("wrote ita.una".to_owned());
    }
    finish_resolved(
        command,
        &written,
        &changes,
        &lines,
        &diagnostics,
        sources,
        out,
    )
}

/// The review of rule 25: each package the new lock adds, changes or drops against the lock it
/// replaces, compared by name and origin, a change of its version, digest, ceiling, trusted
/// modules or consented digest included. What each one's code uses comes from the new graph,
/// checked at tier 0, only when something changed. O(p log p) for p packages, plus the check.
fn review(
    package: &crate::package::Package,
    existing: Option<&una_project::ita::ItaLock>,
    entries: &[Locked],
) -> Vec<ChangeDoc> {
    struct Old {
        version: String,
        digest: Option<String>,
        ceiling: String,
        trusted: Vec<String>,
        consented: Option<String>,
    }
    // The packages of the lock it replaces, by name and origin.
    let mut old: BTreeMap<(String, String), Old> = BTreeMap::new();
    for locked in existing.map_or(&[][..], |lock| lock.read.packages.as_slice()) {
        let origin = match locked.from.as_ref().map(|from| &from.value) {
            Some(ita::LockOrigin::Path(dir)) => format!("path {}", dir.trim_end_matches('/')),
            Some(ita::LockOrigin::Workspace(dir)) => format!("workspace {dir}"),
            Some(ita::LockOrigin::Source(source)) => source.clone(),
            None => String::new(),
        };
        let ceiling = locked
            .granted
            .iter()
            .find(|g| g.identity.is_none())
            .map(|g| canonical_ceiling(&g.may))
            .unwrap_or_default();
        let mut trusted: Vec<String> = locked.trusted.iter().map(|m| m.value.clone()).collect();
        trusted.sort();
        old.insert(
            (locked.name.value.to_string(), origin),
            Old {
                version: locked.version.value.clone(),
                digest: locked.digest.as_ref().map(|d| d.value.clone()),
                ceiling,
                trusted,
                consented: locked.consented.as_ref().map(|c| c.value.clone()),
            },
        );
    }
    let mut changes: Vec<ChangeDoc> = Vec::new();
    for entry in entries {
        let origin = format!("path {}", entry.dir);
        let key = (entry.name.clone(), origin.clone());
        let version = entry.version.to_string();
        let change = match old.remove(&key) {
            None => ChangeDoc {
                change: "add",
                package: entry.name.clone(),
                origin,
                version_from: None,
                version_to: Some(version),
                digest_from: None,
                digest_to: Some(entry.digest.clone()),
                ceiling_from: String::new(),
                ceiling_to: entry.ceiling.clone(),
                trusted_from: Vec::new(),
                trusted_to: entry.trusted.clone(),
                consented_from: None,
                consented_to: entry.consented.clone(),
                authority: Vec::new(),
            },
            Some(before) => {
                let same = before.version == version
                    && before.digest.as_deref() == Some(entry.digest.as_str())
                    && before.ceiling == entry.ceiling
                    && before.trusted == entry.trusted
                    && before.consented == entry.consented;
                if same {
                    continue;
                }
                ChangeDoc {
                    change: "change",
                    package: entry.name.clone(),
                    origin,
                    version_from: Some(before.version),
                    version_to: Some(version),
                    digest_from: before.digest,
                    digest_to: Some(entry.digest.clone()),
                    ceiling_from: before.ceiling,
                    ceiling_to: entry.ceiling.clone(),
                    trusted_from: before.trusted,
                    trusted_to: entry.trusted.clone(),
                    consented_from: before.consented,
                    consented_to: entry.consented.clone(),
                    authority: Vec::new(),
                }
            }
        };
        changes.push(change);
    }
    for ((name, origin), before) in old {
        changes.push(ChangeDoc {
            change: "drop",
            package: name,
            origin,
            version_from: Some(before.version),
            version_to: None,
            digest_from: before.digest,
            digest_to: None,
            ceiling_from: before.ceiling,
            ceiling_to: String::new(),
            trusted_from: before.trusted,
            trusted_to: Vec::new(),
            consented_from: before.consented,
            consented_to: None,
            authority: Vec::new(),
        });
    }
    if changes.iter().any(|c| c.change != "drop") {
        let analysis = crate::on_large_stack(|| crate::analyze(package, None));
        let (manifest, _) = crate::package::project(package);
        let dependencies = crate::report::dependencies(package, &manifest, &analysis);
        let uses: BTreeMap<(String, String), Vec<String>> = dependencies
            .iter()
            .filter_map(|d| {
                let (name, dir, authority) = d.uses();
                Some((
                    (name.to_owned(), format!("path {}", dir?)),
                    authority.to_vec(),
                ))
            })
            .collect();
        for change in changes.iter_mut().filter(|c| c.change != "drop") {
            let key = (change.package.clone(), change.origin.clone());
            if let Some(authority) = uses.get(&key) {
                change.authority = authority.clone();
            }
        }
    }
    changes.sort_by(|a, b| (&a.package, &a.origin).cmp(&(&b.package, &b.origin)));
    changes
}

/// [`finish`] for a command that resolves: its `una.ita/1` document has `changes` too.
fn finish_resolved(
    command: &str,
    written: &[&str],
    changes: &[ChangeDoc],
    lines: &[String],
    diagnostics: &[Diagnostic],
    sources: &SourceDb,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    let status = if diagnostics.iter().any(|d| d.code == Code::U8001) {
        ExitStatus::Internal
    } else if diagnostics.is_empty() {
        ExitStatus::Success
    } else {
        ExitStatus::Errors
    };
    if out.json {
        let document = ResolveDocument {
            schema: "una.ita/1",
            command,
            written,
            changes,
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

/// `una.ita/1` of `una ita tree`, with `schema` first.
#[derive(Serialize)]
struct TreeDocument<'a> {
    schema: &'static str,
    command: &'static str,
    root: &'a str,
    packages: &'a [crate::report::DependencyDoc],
    diagnostics: &'a [Diagnostic],
}

/// `una ita tree [path]` (ADR 0122 rule 57): the program's packages, each with its source,
/// version, the ceilings its dependents write, its effective ceiling, the authority it uses and
/// the trust it adds, after checking the program at tier 0. With `--json`, one `una.ita/1`
/// document whose `packages` are the report's `dependencies`. No network.
pub(crate) fn tree_command(path: &Path, out: Output<'_>) -> io::Result<ExitStatus> {
    let package = match crate::package::load(path) {
        Ok(package) => package,
        Err(error) => {
            writeln!(out.stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let analysis = crate::on_large_stack(|| crate::analyze(&package, None));
    let (manifest, _) = crate::package::project(&package);
    let packages = crate::report::dependencies(&package, &manifest, &analysis);
    let root = format!(
        "{} {}",
        manifest
            .name
            .as_ref()
            .map_or("", |name| name.value.as_str()),
        manifest
            .version
            .as_ref()
            .map_or("", |version| version.value.as_str())
    );
    let status = if analysis.diagnostics.is_empty() {
        ExitStatus::Success
    } else {
        ExitStatus::Errors
    };
    if out.json {
        let document = TreeDocument {
            schema: "una.ita/1",
            command: "tree",
            root: &root,
            packages: &packages,
            diagnostics: &analysis.diagnostics,
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
            render_human(&analysis.diagnostics, &package.sources.texts(), out.color)
        )?;
        writeln!(out.stdout, "{root}")?;
        for dependency in &packages {
            writeln!(
                out.stdout,
                "  {}",
                crate::report::dependency_line(dependency)
            )?;
        }
    }
    Ok(status)
}

/// Reports an internal error of an edit of `ita.una` (U8001), exit code 3.
fn internal_error(out: Output<'_>, what: &str) -> io::Result<ExitStatus> {
    writeln!(
        out.stderr,
        "internal compiler error (U8001): {what} in ita.una, though it parsed; please report it"
    )?;
    Ok(ExitStatus::Internal)
}

/// `una ita remove <name> [path]` (ADR 0122 rule 57): removes the need of `name` from
/// `ita.una`, its indented lines with it, and the `needs` section when it empties; then
/// resolves as `una ita lock` does, and writes the lock, then `ita.una`, only when the
/// resolution succeeds, each through a temporary file and a rename. The edit is made on the
/// decoded text, `\r\n` read as `\n`, which the ranges of the grammar index. A name the root
/// does not need is a usage error. No network.
pub(crate) fn remove_command(name: &str, path: &Path, out: Output<'_>) -> io::Result<ExitStatus> {
    let manifest_path = path.join("ita.una");
    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => text,
        Err(error) => return unreadable(&manifest_path, &error, out),
    };
    let (read, errors) = ita::parse_manifest(&text);
    if !errors.is_empty() {
        writeln!(
            out.stderr,
            "error: ita.una does not parse; run `una check` to see why, then remove the need"
        )?;
        return Ok(ExitStatus::Usage);
    }
    let Some(need) = read
        .needs
        .iter()
        .find(|need| need.name.value.to_string() == name)
    else {
        writeln!(out.stderr, "error: the package does not need `{name}`")?;
        return Ok(ExitStatus::Usage);
    };
    let lines = ita::lines(&text);
    // The ranges of the grammar index the decoded text, so the edit is made on it.
    let text = lines.text.as_str();
    let Some(section) = lines.lines.iter().find(|line| {
        line.tokens
            .first()
            .is_some_and(|t| t.item == ita::Item::Word("needs".to_owned()))
    }) else {
        return internal_error(out, "the need was found but not its `needs` section");
    };
    // The byte range of whole lines to remove: the need's line and its children, or the whole
    // section when the need is its only one.
    let line_start = |offset: usize| {
        text.get(..offset)
            .and_then(|head| head.rfind('\n'))
            .map_or(0, |at| at + 1)
    };
    let line_end = |offset: usize| {
        text.get(offset..)
            .and_then(|tail| tail.find('\n'))
            .map_or(text.len(), |at| offset + at + 1)
    };
    let last_of = |line: &ita::Line| {
        let mut last = line.range.end;
        let mut stack: Vec<&ita::Line> = line.children.iter().collect();
        while let Some(child) = stack.pop() {
            last = last.max(child.range.end);
            stack.extend(child.children.iter());
        }
        last
    };
    let (from, to) = if section.children.len() == 1 {
        let mut from = line_start(section.range.start);
        // Take the blank line before the section with it.
        if text.get(..from).is_some_and(|head| head.ends_with("\n\n")) {
            from -= 1;
        }
        (from, line_end(last_of(section)))
    } else {
        let Some(child) = section
            .children
            .iter()
            .find(|c| c.range.start == need.range.start)
        else {
            return internal_error(out, "the need's line was not found under `needs`");
        };
        (line_start(child.range.start), line_end(last_of(child)))
    };
    let mut edited = String::with_capacity(text.len());
    edited.push_str(text.get(..from).unwrap_or(""));
    edited.push_str(text.get(to..).unwrap_or(""));
    // The edit removes exactly this need, and nothing else.
    let (after, _) = ita::parse_manifest(&edited);
    let kept: Vec<String> = read
        .needs
        .iter()
        .map(|n| n.name.value.to_string())
        .filter(|n| n != name)
        .collect();
    let left: Vec<String> = after.needs.iter().map(|n| n.name.value.to_string()).collect();
    if left != kept {
        return internal_error(out, "removing the need would change other lines");
    }
    resolve_and_lock(
        path,
        LockOptions {
            command: "remove",
            edited: Some(edited),
            ..LockOptions::lock(false, false, &[], &[])
        },
        out,
    )
}

/// `una ita vendor [path]` (ADR 0122 rule 32): copies each registry package of `ita.lock` from
/// Itaoca into `vendor/ita/<source>/<release>/` of the package, with its provenance beside it in
/// `<release>.provenance`, and loads the copy verified as every command does (rule 17). A copy
/// already there that verifies is kept; one whose files do not verify is U7010 and is left as
/// it is; a package that Itaoca does not hold is U7011. Path dependencies are not copied: they
/// are loaded from their directory. An invalid `ita.una` or `ita.lock` is reported and nothing
/// is written. No connection is opened. O(bytes of the packages).
pub(crate) fn vendor_command(
    path: &Path,
    itaoca: Option<&una_project::itaoca::Itaoca>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
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
    let Some(lock_id) = package.lock else {
        diagnostics.extend(located(
            sources,
            manifest_id,
            vec![ProjectError {
                code: Code::U7009,
                range: 0..0,
                message: "the package has no ita.lock; run `una ita lock`".to_owned(),
            }],
        ));
        return finish("vendor", &[], &[], &diagnostics, sources, out);
    };
    let (lock, errors) = parse_ita_lock(sources.file(lock_id).text());
    diagnostics.extend(located(sources, lock_id, errors));
    if !diagnostics.is_empty() {
        return finish("vendor", &[], &[], &diagnostics, sources, out);
    }
    let mut written: Vec<String> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut problems: Vec<ProjectError> = Vec::new();
    for locked in &lock.read.packages {
        if !matches!(
            &locked.from,
            Some(ita::At {
                value: ita::LockOrigin::Source(_),
                ..
            })
        ) {
            continue;
        }
        match una_project::vendor::vendor_locked(
            path,
            itaoca,
            &lock.read,
            manifest.ita.as_deref(),
            locked,
        ) {
            Ok(paths) => {
                if !paths.is_empty() {
                    lines.push(format!(
                        "vendored {} {}",
                        locked.name.value, locked.version.value
                    ));
                }
                written.extend(paths);
            }
            Err(problem) => {
                let (code, message) = match problem {
                    una_project::vendor::Problem::Absent(message) => (Code::U7011, message),
                    una_project::vendor::Problem::Refused(message) => (Code::U7010, message),
                };
                problems.push(ProjectError {
                    code,
                    range: locked.range.clone(),
                    message,
                });
            }
        }
    }
    diagnostics.extend(located(sources, lock_id, problems));
    if lines.is_empty() && diagnostics.is_empty() {
        lines.push("vendor/ita is up to date".to_owned());
    }
    let written: Vec<&str> = written.iter().map(String::as_str).collect();
    finish("vendor", &written, &lines, &diagnostics, sources, out)
}

/// `una ita update [<name>…] [--cooldown <hours>] [--now] [--dry-run] [--accept-…] [path]`
/// (ADR 0122 rules 25 and 57): raises the named needs, or all, and writes the lock. A named
/// need is resolved again; every other need that the lock records keeps its entry as recorded,
/// its consents included. A name the root does not need, or an `ita.una` that does not parse,
/// is a usage error. In this release a path package is locked as its directory holds it, so
/// `update` locks it again with the review of rule 25; a registry need cannot be raised
/// offline, since the per-name state of a resolution point is not held (U7013), and nothing is
/// written. No network.
pub(crate) fn update_command<'a>(
    names: &'a [String],
    path: &Path,
    options: LockOptions<'a>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    let manifest_path = path.join("ita.una");
    if !names.is_empty() {
        let text = match std::fs::read_to_string(&manifest_path) {
            Ok(text) => text,
            Err(error) => return unreadable(&manifest_path, &error, out),
        };
        let (read, errors) = ita::parse_manifest(&text);
        if !errors.is_empty() {
            writeln!(
                out.stderr,
                "error: ita.una does not parse; run `una check` to see why, then update"
            )?;
            return Ok(ExitStatus::Usage);
        }
        for name in names {
            if !read
                .needs
                .iter()
                .any(|need| need.name.value.to_string() == *name)
            {
                writeln!(out.stderr, "error: the package does not need `{name}`")?;
                return Ok(ExitStatus::Usage);
            }
        }
    }
    let only = (!names.is_empty()).then_some(names);
    resolve_and_lock(path, LockOptions { only, ..options }, out)
}

/// Whether `word` is a lower-case identifier of a project file: `[a-z][a-z0-9_]*`, at most 64
/// bytes.
fn is_lower_word(word: &str) -> bool {
    word.len() <= 64
        && word.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && word
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The sections of `ita.una` that come after `needs`, in the canonical order (ADR 0122 rule 5).
const AFTER_NEEDS: [&str; 9] = [
    "replace",
    "trust",
    "labels",
    "trusted",
    "escapes",
    "boundaries",
    "contracts",
    "run",
    "drive",
];

/// `text` with `line` added as the last need: at the end of the `needs` section, or in a new
/// one before the first section that follows it in the canonical order, or at the end.
fn with_need(text: &str, line: &str) -> String {
    let read = ita::lines(text);
    let text = read.text.as_str();
    let line_start = |offset: usize| {
        text.get(..offset)
            .and_then(|head| head.rfind('\n'))
            .map_or(0, |at| at + 1)
    };
    let line_end = |offset: usize| {
        text.get(offset..)
            .and_then(|tail| tail.find('\n'))
            .map_or(text.len(), |at| offset + at + 1)
    };
    fn first_word(top: &ita::Line) -> Option<&str> {
        match top.tokens.first().map(|t| &t.item) {
            Some(ita::Item::Word(word)) => Some(word.as_str()),
            _ => None,
        }
    }
    let (at, insert) = if let Some(section) = read
        .lines
        .iter()
        .find(|top| first_word(top) == Some("needs"))
    {
        let mut last = section.range.end;
        let mut stack: Vec<&ita::Line> = section.children.iter().collect();
        while let Some(child) = stack.pop() {
            last = last.max(child.range.end);
            stack.extend(child.children.iter());
        }
        (line_end(last), format!("{line}\n"))
    } else if let Some(next) = read
        .lines
        .iter()
        .find(|top| first_word(top).is_some_and(|word| AFTER_NEEDS.contains(&word)))
    {
        let start = next
            .comments_before
            .iter()
            .map(|comment| comment.start)
            .chain([next.range.start])
            .min()
            .unwrap_or(next.range.start);
        (line_start(start), format!("needs\n{line}\n\n"))
    } else {
        let gap = if text.ends_with('\n') { "\n" } else { "\n\n" };
        (text.len(), format!("{gap}needs\n{line}\n"))
    };
    let mut edited = String::with_capacity(text.len() + insert.len());
    edited.push_str(text.get(..at).unwrap_or(text));
    // A need added at the end of a file without a final line feed starts on its own line.
    if at == text.len() && !text.ends_with('\n') && insert.starts_with("  ") {
        edited.push('\n');
    }
    edited.push_str(&insert);
    edited.push_str(text.get(at..).unwrap_or(""));
    edited
}

/// `una ita add <name> [<version>] [--as <local>] [--from <source>] [--now] [--dry-run]
/// [--accept-…] [path]` (ADR 0122 rules 25 and 57): adds a need to `ita.una` and locks, both
/// written only when the resolution succeeds. It writes no `may` and no `trust` line. Each word
/// is checked before it is written (a usage error otherwise), and the edited `ita.una` is read
/// by the grammar like any other (U7008). The need is a registry's, which this release cannot
/// resolve offline (U7013): nothing is written. No network.
#[allow(clippy::too_many_arguments)]
pub(crate) fn add_command(
    name: &str,
    version: Option<&str>,
    local: Option<&str>,
    from: Option<&str>,
    path: &Path,
    options: LockOptions<'_>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    let manifest_path = path.join("ita.una");
    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => text,
        Err(error) => return unreadable(&manifest_path, &error, out),
    };
    let name_ok = match name.split_once('/') {
        Some((owner, short)) => is_lower_word(owner) && is_lower_word(short),
        None => is_lower_word(name),
    };
    if !name_ok {
        writeln!(
            out.stderr,
            "error: `{name}` is not a package's name: write `name` or `owner/name` in lower case"
        )?;
        return Ok(ExitStatus::Usage);
    }
    for (flag, word) in [("--as", local), ("--from", from)] {
        if let Some(word) = word
            && !is_lower_word(word)
        {
            writeln!(
                out.stderr,
                "error: `{word}` after {flag} is not a lower-case name"
            )?;
            return Ok(ExitStatus::Usage);
        }
    }
    if let Some(version) = version {
        let written = version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
        if !written || Version::parse(version).is_err() {
            writeln!(
                out.stderr,
                "error: `{version}` is not a version: write `2.1` or `2.1.0`"
            )?;
            return Ok(ExitStatus::Usage);
        }
    }
    let (before, errors) = ita::parse_manifest(&text);
    if !errors.is_empty() {
        writeln!(
            out.stderr,
            "error: ita.una does not parse; run `una check` to see why, then add the need"
        )?;
        return Ok(ExitStatus::Usage);
    }
    let Some(version) = version else {
        // Rule 25 takes the newest release of the newest major past the cooldown, which only
        // the per-name state of a resolution point tells.
        let source = from.unwrap_or(una_project::ita::ITATY);
        let mut sources = SourceDb::new();
        let id = sources.add("ita.una".to_owned(), text.as_bytes());
        let diagnostics = located(
            &sources,
            id,
            vec![ProjectError {
                code: Code::U7013,
                range: 0..0,
                message: format!(
                    "the newest release of `{name}` cannot be found offline, since the per-name state of `{source}` at a resolution point is not held"
                ),
            }],
        );
        return finish_resolved("add", &[], &[], &[], &diagnostics, &sources, out);
    };
    let mut line = format!("  {name} {version}");
    if let Some(local) = local {
        line.push_str(&format!(" as {local}"));
    }
    if let Some(from) = from {
        line.push_str(&format!(" from {from}"));
    }
    let edited = with_need(&text, &line);
    // The edit adds exactly one need, and nothing else.
    let (after, _) = ita::parse_manifest(&edited);
    if after.needs.len() != before.needs.len() + 1 {
        writeln!(
            out.stderr,
            "error: the need cannot be added to this ita.una; write it under `needs` by hand"
        )?;
        return Ok(ExitStatus::Usage);
    }
    resolve_and_lock(
        path,
        LockOptions {
            command: "add",
            edited: Some(edited),
            ..options
        },
        out,
    )
}

/// `una ita verify [path]` (ADR 0122 rule 57): re-verifies each registry package of `ita.lock`
/// in the vendor directory and in Itaoca, wherever it is, against the lock and the provenance
/// stored beside it (rule 17): first its presence (U7011 when it is in neither place), then
/// its files read again with no link followed, their digest, and the stored statement,
/// checkpoint and root (U7010 for a copy that does not verify). Even when every copy checks,
/// the package is not reported verified: the owner's key chain and its name map proof, which
/// show that the signing key was valid (rules 36.1 and 36.3), are not held until the registry
/// client supplies them, so each package is unavailable (U7013) and the command exits with
/// code 1. Nothing is written or repaired. Path packages are the root author's working tree,
/// which `una ita lock --check` compares. No network. O(bytes of the packages).
pub(crate) fn verify_command(
    path: &Path,
    itaoca: Option<&una_project::itaoca::Itaoca>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    use una_project::vendor::{self, Problem};

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
        writeln!(
            out.stderr,
            "internal compiler error (U8001): `{}` was loaded without its ita.una; please report it",
            path.display()
        )?;
        return Ok(ExitStatus::Internal);
    };
    let (manifest, errors) = parse_ita_manifest(sources.file(manifest_id).text());
    let mut diagnostics = located(sources, manifest_id, errors);
    let Some(lock_id) = package.lock else {
        diagnostics.extend(located(sources, manifest_id, vec![missing_lock()]));
        return finish("verify", &[], &[], &diagnostics, sources, out);
    };
    let (lock, errors) = parse_ita_lock(sources.file(lock_id).text());
    diagnostics.extend(located(sources, lock_id, errors));
    if !diagnostics.is_empty() {
        return finish("verify", &[], &[], &diagnostics, sources, out);
    }
    let mut lines: Vec<String> = Vec::new();
    let mut problems: Vec<ProjectError> = Vec::new();
    let mut registry_packages = 0usize;
    for locked in &lock.read.packages {
        if !matches!(
            &locked.from,
            Some(ita::At {
                value: ita::LockOrigin::Source(_),
                ..
            })
        ) {
            continue;
        }
        registry_packages += 1;
        let what = format!("{} {}", locked.name.value, locked.version.value);
        let mut refuse = |code: Code, message: String| {
            problems.push(ProjectError {
                code,
                range: locked.range.clone(),
                message,
            });
        };
        // Presence first, as every load checks it (rule 17): in neither place is U7011.
        match vendor::present(path, itaoca, locked) {
            Ok(true) => {}
            Ok(false) => {
                refuse(
                    Code::U7011,
                    format!(
                        "the locked package `{what}` is in neither the vendor directory nor Itaoca"
                    ),
                );
                continue;
            }
            Err(Problem::Refused(message) | Problem::Absent(message)) => {
                refuse(Code::U7010, message);
                continue;
            }
        }
        let expected = match vendor::expected(&lock.read, manifest.ita.as_deref(), locked) {
            Ok(expected) => expected,
            Err(message) => {
                refuse(Code::U7010, message);
                continue;
            }
        };
        let mut places: Vec<&str> = Vec::new();
        let mut failed = false;
        match vendor::vendored(path, &expected) {
            Ok(Some(_)) => places.push("vendor/ita"),
            Ok(None) => {}
            Err(Problem::Refused(message) | Problem::Absent(message)) => {
                refuse(Code::U7010, message);
                failed = true;
            }
        }
        if let Some(itaoca) = itaoca {
            match vendor::stored(itaoca, &expected) {
                Ok(Some(_)) => places.push("Itaoca"),
                Ok(None) => {}
                Err(Problem::Refused(message) | Problem::Absent(message)) => {
                    refuse(Code::U7010, message);
                    failed = true;
                }
            }
        }
        if failed {
            continue;
        }
        if places.is_empty() {
            refuse(
                Code::U7011,
                format!(
                    "the locked package `{what}` is in neither the vendor directory nor Itaoca"
                ),
            );
            continue;
        }
        // Every copy checks against the lock, and still the package is not verified: the key
        // that signed its statement is not shown valid (rules 36.1 and 36.3).
        refuse(
            Code::U7013,
            format!(
                "`{what}` is not verified: its files and provenance in {} match the lock, but the owner's key chain and its name map proof are not held until the registry client supplies them",
                places.join(" and ")
            ),
        );
    }
    diagnostics.extend(located(sources, lock_id, problems));
    if registry_packages == 0 && diagnostics.is_empty() {
        lines.push("no registry package is locked".to_owned());
    }
    finish("verify", &[], &lines, &diagnostics, sources, out)
}
