//! `una ita` (ADR 0122 rule 57), the package manager of edition `0.4`, and `una fmt` of its
//! project files (rule 5).
//!
//! `una ita lock`, `update` and `add` write the solver section, kept from the lock it replaces or
//! the default one, then each path dependency and each registry release of the program, resolved
//! by minimal version selection: a path dependency as its directory holds it, and a source of
//! `sources` from its registry, read from its mirrors or its own URL and verified by the
//! registry client ([`crate::registry`]). A need of `itaty`, whose root is not built into this release, is
//! refused (U7013), and one of a workspace member is refused (U7008): never left unresolved, and
//! never resolved from a partial view. `una ita fetch` fills Itaoca with the locked releases,
//! verified. No command here opens a network connection or starts a program.

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

/// What `una ita lock`, `update` and `add` ask of a resolution (ADR 0122 rule 25).
pub(crate) struct LockOptions<'a> {
    /// The command, as `una.ita/1` names it.
    pub(crate) command: &'static str,
    /// `--check`: write nothing, and report where the lock is not the one written (U7009).
    pub(crate) check: bool,
    /// `--dry-run`: print the review, and write nothing.
    pub(crate) dry_run: bool,
    /// `--accept-authority`: record a ceiling wider than the one the lock records.
    pub(crate) accept_authority: bool,
    /// `--accept-trust <package>`: record consent to the trust of these packages.
    pub(crate) accept_trust: &'a [String],
    /// The cooldown of rule 25, in seconds: 72 hours by default, 0 with `--now`.
    pub(crate) cooldown: u64,
    /// `ita.una` as `una ita add` edits it: read in place of the file, and written, with the
    /// lock, only when the resolution succeeds.
    pub(crate) edited: Option<String>,
    /// `--accept-key <package>`: record a change of these packages' owners' keys (rule 25).
    pub(crate) accept_key: &'a [String],
    /// Itaoca, whose stored checkpoints are the registries' floors (rule 39).
    pub(crate) itaoca: Option<&'a una_project::itaoca::Itaoca>,
    /// The local clock, in seconds, for the freshness of a registry's checkpoint (rule 39).
    pub(crate) now: u64,
    /// `una ita update`: the needs to raise to the newest release past the cooldown, by name;
    /// all when empty.
    pub(crate) raise: Option<&'a [String]>,
}

impl<'a> LockOptions<'a> {
    /// The options of `una ita lock`.
    pub(crate) fn lock(
        check: bool,
        dry_run: bool,
        accept_authority: bool,
        accept_trust: &'a [String],
    ) -> LockOptions<'a> {
        LockOptions {
            command: "lock",
            check,
            dry_run,
            accept_authority,
            accept_trust,
            cooldown: una_project::resolve::COOLDOWN,
            edited: None,
            accept_key: &[],
            itaoca: None,
            now: 0,
            raise: None,
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
    /// What its code uses in the new graph, checked at tier 0: the union of the `uses` of its
    /// functions and handlers.
    authority: Vec<String>,
}

impl ChangeDoc {
    /// The review line: `add util 0.2.0 (path deps/util): digest sha256:...; may Console[print];
    /// uses Console[print]`.
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

/// A package as the resolution locks it.
struct Locked {
    name: String,
    version: Version,
    /// Its origin as the review writes it: `path deps/util`, or a registry source.
    origin: String,
    digest: String,
    ceiling: String,
    block: String,
    /// What a registry release's statement says its code needs; a path package's comes from
    /// checking the new graph.
    authority: Option<Vec<String>>,
}

/// `una ita lock [--check] [--dry-run] [--accept-authority] [--accept-trust <p>] [path]`
/// (ADR 0122 rules 14 to 17, 23 and 25).
pub(crate) fn lock_command(
    path: &Path,
    options: LockOptions<'_>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    resolve_and_lock(path, options, out)
}

/// Resolves the package at `path` and writes `ita.lock` through a temporary file and a rename.
///
/// Resolution is minimal version selection (rules 23 and 24) over each path dependency as its
/// directory holds it, one release of its identity, and over the releases of each source of
/// `sources` that the root needs, read from the registry at a fresh checkpoint and verified
/// (rules 35 to 39, [`una_project::client`]); `--check` reads them at the lock's own resolution
/// point. A source is read from the first of its places, the mirrors Itaoca's `mirrors.una`
/// lists then its own URL, whose copy verifies; one with no place this release can read (an
/// `https://` location has no transport yet) is U7013, as a need of `itaty`, whose root is not
/// built into this release. A registry that fails verification is U7010. A workspace member is not loaded
/// (U7008). An avoided version that no later release replaces, and a bound of rule 23
/// exceeded, are U7013.
///
/// The lock's solver section is the existing lock's, or the release's default; then each
/// registry's resolution point, then each package, with its version, origin, content digest,
/// and the ceiling the root grants it; a registry package with its key, log index and needs.
/// Consent (rule 25): a ceiling wider than the one the lock records needs `--accept-authority`,
/// a trust that no consent covers for the package's content as it is now needs
/// `--accept-trust <package>`, and a change of a locked registry package's owner's keys needs
/// `--accept-key <package>` (U7014); each refusal writes nothing. A locked release replaced by
/// one that needs more authority or trust breaks rule 26 (U7010). Every change is printed as a
/// review before it is written. `--dry-run` prints the review and writes nothing; `--check`
/// writes nothing and reports where the lock is not the one this command writes (U7009). The
/// checkpoint of each registry read is stored in Itaoca as its floor when the lock is written.
/// Costs: O(bytes of the dependencies) for their digests, the registry client's costs per name
/// read, and O((n + p) log p) for n needs and p packages to resolve and sort.
fn resolve_and_lock(
    path: &Path,
    options: LockOptions<'_>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    use std::cell::RefCell;

    use una_project::client::{Online, Point, Registry};
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
        return finish_resolved(command, &[], &[], &[], &diagnostics, sources, out);
    };
    let mirrors = match crate::registry::load_mirrors(options.itaoca) {
        Ok(mirrors) => mirrors,
        Err(errors) => {
            diagnostics.extend(located(sources, manifest_id, errors));
            return finish_resolved(command, &[], &[], &[], &diagnostics, sources, out);
        }
    };
    let root = read
        .name
        .as_ref()
        .map(|name| name.value.to_string())
        .unwrap_or_default();
    let existing_lock = existing.as_ref().map(|(_, lock, _)| lock);
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
    // The registry needs of the root, by identity, with the index of the need.
    let mut direct: BTreeMap<Identity, usize> = BTreeMap::new();
    for (index_of_need, need) in read.needs.iter().enumerate() {
        let refuse = |code: Code, message: String| ProjectError {
            code,
            range: need.range.clone(),
            message,
        };
        let source = match need.origin.as_ref().map(|origin| &origin.value) {
            Some(ita::Origin::Path(_)) => None,
            Some(ita::Origin::Workspace) => {
                diagnostics.extend(located(
                    sources,
                    manifest_id,
                    vec![refuse(
                        Code::U7008,
                        format!(
                            "this release does not load workspace members, so `{}` cannot be locked",
                            need.name.value
                        ),
                    )],
                ));
                continue;
            }
            Some(ita::Origin::Source(source)) => Some(source.clone()),
            None => Some(una_project::ita::ITATY.to_owned()),
        };
        if let Some(source) = source {
            if source == una_project::ita::ITATY {
                // The root of `itaty` is to be built into `una` (rule 35); until it is, none of
                // its packages can be verified, so its per-name state is refused as a whole.
                let reason = format!(
                    "`{}` is a package of `itaty`, whose root metadata is not built into this release, so the per-name state of `itaty` at a resolution point cannot be verified; this release resolves path dependencies and the sources of `sources`",
                    need.name.value
                );
                index.refuse(&source, reason.clone());
                diagnostics.extend(located(
                    sources,
                    manifest_id,
                    vec![refuse(Code::U7013, reason)],
                ));
                continue;
            }
            // A need of a registry: its least version, on its major line (rule 22). One
            // without a version is U7008 already.
            let Some(least) = need
                .version
                .as_ref()
                .and_then(|v| Version::parse(&v.value).ok())
                .map(|written| written.version)
            else {
                continue;
            };
            let identity = Identity {
                source: source.clone(),
                name: need.name.value.to_string(),
                line: least.line(),
            };
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
                source,
                name: identity.name.clone(),
                version: least,
            });
            need_ranges.push((identity.clone(), need.range.clone()));
            direct.insert(identity, index_of_need);
            continue;
        }
        let Some(dependency) = package
            .dependencies
            .iter()
            .find(|d| d.need == index_of_need && d.registry.is_none())
        else {
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
            continue;
        };
        let (settings, errors) = parse_ita_manifest(sources.file(dependency_manifest).text());
        let invalid = !errors.is_empty();
        diagnostics.extend(located(sources, dependency_manifest, errors));
        let problems = crate::package::dependency_problems(need, &settings, &manifest, true);
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
        let digest = match crate::package::dependency_digest(&package, dependency) {
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
        diagnostics.extend(located(
            sources,
            manifest_id,
            ceiling_consent(need, existing_lock, &root, options.accept_authority, repair),
        ));
        // Trust (rules 10, 25 and 53): consent to a dependency's trusted modules is pinned to
        // its whole content, and recorded only with `--accept-trust <name>`.
        if !settings.trusted.is_empty() {
            let name = need.name.value.to_string();
            let mut trusted: Vec<&str> =
                settings.trusted.iter().map(|t| t.value.as_str()).collect();
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
            let consented = existing_lock.is_some_and(|lock| {
                lock.read.packages.iter().any(|p| {
                    p.name.value == need.name.value
                        && p.consented.as_ref().is_some_and(|c| c.value == digest)
                        && trusted
                            .iter()
                            .all(|t| p.trusted.iter().any(|m| m.value == *t))
                })
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
            if !consented && !options.accept_trust.contains(&name) {
                diagnostics.extend(located(
                    sources,
                    manifest_id,
                    vec![ProjectError {
                        code: Code::U7012,
                        range: need.range.clone(),
                        message: format!(
                            "the trust of `{name}` is not consented for its content as it is now; review its trusted modules and the code they run, then run `una ita {repair} --accept-trust {name}`"
                        ),
                    }],
                ));
                continue;
            }
            block.push_str(&format!(
                "  consented \"{digest}\"\n  trusted modules {}\n",
                trusted.join(", ")
            ));
        }
        // One release of the identity, as the directory holds it; its own needs are refused
        // by `dependency_problems` in this release, so it has none.
        let identity = Identity {
            source: format!("path {}", dependency.dir),
            name: need.name.value.to_string(),
            line: version.line(),
        };
        index.add(
            identity.clone(),
            Release {
                version: version.clone(),
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
            version: version.clone(),
        });
        need_ranges.push((identity.clone(), need.range.clone()));
        entries.push(Locked {
            name: need.name.value.to_string(),
            version,
            origin: identity.source,
            digest,
            ceiling,
            block,
            authority: None,
        });
    }
    if !diagnostics.is_empty() {
        return finish_resolved(command, &[], &[], &[], &diagnostics, sources, out);
    }
    // The registries the root needs, each read from the first of its places, its mirrors then
    // its own URL, whose copy verifies (rules 35 to 40).
    let first_need = |source: &str| {
        need_ranges
            .iter()
            .find(|(identity, _)| identity.source == source)
            .map_or(0..0, |(_, range)| range.clone())
    };
    let registry_sources: BTreeSet<String> =
        direct.keys().map(|identity| identity.source.clone()).collect();
    let mut places: BTreeMap<String, Vec<crate::registry::Place>> = BTreeMap::new();
    for source in &registry_sources {
        match crate::registry::declared(read, source) {
            Some(declared) => {
                places.insert(
                    source.clone(),
                    crate::registry::places_of(declared, &mirrors),
                );
            }
            None => diagnostics.extend(located(
                sources,
                manifest_id,
                vec![ProjectError {
                    code: Code::U7013,
                    range: first_need(source),
                    message: format!("`{source}` is not a source of `sources`"),
                }],
            )),
        }
    }
    if !diagnostics.is_empty() {
        return finish_resolved(command, &[], &[], &[], &diagnostics, sources, out);
    }
    let locked_point = |source: &str| {
        existing_lock.and_then(|lock| {
            lock.read
                .registries
                .iter()
                .find(|r| r.source.value == source)
                .map(|r| (r.checkpoint_size.value, r.checkpoint_hash.value.clone()))
        })
    };
    let mut notes: Vec<String> = Vec::new();
    let mut registries: BTreeMap<String, RefCell<Registry<'_>>> = BTreeMap::new();
    for (source, list) in &places {
        let Some(declared) = crate::registry::declared(read, source) else {
            continue;
        };
        let old = locked_point(source);
        let point = match (&old, options.check) {
            (Some((size, hash)), true) => Point::At {
                size: *size,
                hash: hash.clone(),
            },
            _ => Point::Latest { now: options.now },
        };
        match crate::registry::open_first(declared, list, &point, options.itaoca, old.as_ref()) {
            Ok((registry, skipped)) => {
                notes.extend(skipped);
                registries.insert(source.clone(), RefCell::new(registry));
            }
            Err((code, message)) => diagnostics.extend(located(
                sources,
                manifest_id,
                vec![ProjectError {
                    code,
                    range: first_need(source),
                    message,
                }],
            )),
        }
    }
    if !diagnostics.is_empty() {
        return finish_resolved(command, &[], &[], &[], &diagnostics, sources, out);
    }
    // The cooldown is measured at the earliest of the registries' witness times (rule 36.4).
    if let Some(time) = registries
        .values()
        .filter_map(|registry| registry.try_borrow().ok().map(|r| r.point.time))
        .min()
    {
        policy.now = time;
    }
    let online = Online {
        registries,
        local: index,
    };
    // `una ita update`: each registry need named, or all, raised to the newest release of its
    // identity that may be selected and was logged at least the cooldown before the point.
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    if let Some(names) = options.raise {
        for (identity, need_index) in &direct {
            let Some(need) = read.needs.get(*need_index) else {
                continue;
            };
            if !names.is_empty() && !names.contains(&need.name.value.to_string()) {
                continue;
            }
            let (Some(version), Some(cell)) =
                (&need.version, online.registries.get(&identity.source))
            else {
                continue;
            };
            let state = cell
                .try_borrow_mut()
                .map_err(|_| "a registry is read twice at once".to_owned())
                .and_then(|mut registry| registry.state(&identity.name, identity.line));
            let releases = match state {
                Ok(releases) => releases,
                Err(message) => {
                    diagnostics.extend(located(
                        sources,
                        manifest_id,
                        vec![ProjectError {
                            code: Code::U7010,
                            range: need.range.clone(),
                            message,
                        }],
                    ));
                    continue;
                }
            };
            let Some(need_entry) = root_needs.iter_mut().find(|n| n.identity() == *identity)
            else {
                continue;
            };
            let named_pre = need_entry.version.is_pre();
            let avoided = policy.avoid.get(identity);
            let newest = releases
                .iter()
                .filter(|r| {
                    !r.yanked
                        && !r.advised
                        && (named_pre || !r.version.is_pre())
                        && !avoided.is_some_and(|set| set.contains(&r.version))
                        && (r.threshold_fix
                            || r.logged_at.saturating_add(policy.cooldown) <= policy.now)
                })
                .map(|r| r.version.clone())
                .max();
            if let Some(newest) = newest
                && newest > need_entry.version
            {
                edits.push((version.range.clone(), newest.to_string()));
                need_entry.version = newest;
            }
        }
    }
    if !diagnostics.is_empty() {
        return finish_resolved(command, &[], &[], &[], &diagnostics, sources, out);
    }
    let edited: Option<String> = if edits.is_empty() {
        options.edited.clone()
    } else {
        let mut text = sources.file(manifest_id).text().to_owned();
        edits.sort_by(|a, b| b.0.start.cmp(&a.0.start));
        for (range, version) in &edits {
            if text.get(range.clone()).is_some() {
                text.replace_range(range.clone(), version);
            }
        }
        Some(text)
    };
    // Minimal version selection over what is held and read (rules 23 and 24), within its
    // bounds.
    let resolution = match resolve::resolve(&root_needs, &online, &policy, Bounds::default()) {
        Ok(resolution) => resolution,
        Err(problem) => {
            let at = match &problem {
                Problem::NoRelease { identity, .. }
                | Problem::NoRaise { identity, .. }
                | Problem::SelfNeed(identity) => need_ranges
                    .iter()
                    .find(|(id, _)| id == identity)
                    .map(|(_, range)| range.clone()),
                Problem::Bound(_) | Problem::Index(_) => None,
            };
            // A registry that fails verification while it is read is U7010; every other
            // problem is no resolution.
            let code = match &problem {
                Problem::Index(why) if why.starts_with("the registry `") => Code::U7010,
                _ => Code::U7013,
            };
            let error = ProjectError {
                code,
                range: at.unwrap_or(0..0),
                message: problem.to_string(),
            };
            diagnostics.extend(located(sources, manifest_id, vec![error]));
            return finish_resolved(command, &[], &[], &[], &diagnostics, sources, out);
        }
    };
    // Every path need is in the build list, at the version its directory holds: a path
    // package has one release.
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
    entries.retain(|entry| {
        selected.contains(&(
            entry.name.clone(),
            entry.origin.clone(),
            entry.version.to_string(),
        ))
    });
    // Each registry release of the build list, with its key, log index and needs, its consent
    // and its key checked against the lock it replaces.
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut found: Vec<ProjectError> = Vec::new();
    // Per source and owner, the pending entries whose key change the new lock records as
    // consented to.
    let mut accepted_out: BTreeMap<String, BTreeMap<String, BTreeSet<u64>>> = BTreeMap::new();
    let mut effects_out: BTreeMap<String, BTreeMap<String, BTreeSet<u64>>> = BTreeMap::new();
    // Problems of the lock it replaces, at their lines of it.
    let mut lock_errors: Vec<ProjectError> = Vec::new();
    // Each bad index of an owner is reported once, whatever number of its packages are locked.
    let mut reported: BTreeSet<(String, String, u64)> = BTreeSet::new();
    for (identity, version) in &resolution.list {
        let Some(cell) = online.registries.get(&identity.source) else {
            continue;
        };
        let need = direct.get(identity).and_then(|at| read.needs.get(*at));
        let range = need.map_or(0..0, |need| need.range.clone());
        let problem = |code: Code, message: String| ProjectError {
            code,
            range: range.clone(),
            message,
        };
        let Ok(mut registry) = cell.try_borrow_mut() else {
            found.push(problem(
                Code::U7010,
                "a registry is read twice at once".to_owned(),
            ));
            continue;
        };
        let release = match registry.release(&identity.name, version) {
            Ok(release) => release,
            Err(message) => {
                found.push(problem(Code::U7010, message));
                continue;
            }
        };
        if release.compromised {
            found.push(problem(
                Code::U7010,
                format!(
                    "`{} {version}` counts as signed only with a key of its owner revoked as compromised before it was logged",
                    identity.name
                ),
            ));
            continue;
        }
        let Some(signed) = crate::registry::signed_of(&release) else {
            found.push(problem(
                Code::U7010,
                format!("`{} {version}` has no signer", identity.name),
            ));
            continue;
        };
        let mut block = crate::registry::package_block(&release, &signed);
        // The entries of the owner's chain whose key change the lock it replaces consented to.
        let owner = una_project::client::owner_of(&identity.name).to_owned();
        let old_registry = existing_lock.and_then(|lock| {
            lock.read
                .registries
                .iter()
                .find(|r| r.source.value == identity.source)
        });
        let accepted_line = old_registry
            .and_then(|registry| registry.accepted.iter().find(|a| a.owner.value == owner));
        let effect_line = old_registry
            .and_then(|registry| registry.effects.iter().find(|a| a.owner.value == owner));
        // A recorded index counts only when it is below the old resolution point and is, there,
        // a recovery, a dispute or a co-signer change of the owner's chain: pending for an
        // `accepted` index, in effect for an `effect` one. Any other is U7009 at its line, and
        // counts for nothing.
        let old_size = locked_point(&identity.source).map_or(0, |(size, _)| size);
        let mut recorded: BTreeSet<u64> = BTreeSet::new();
        let mut recorded_effects: BTreeSet<u64> = BTreeSet::new();
        if accepted_line.is_some() || effect_line.is_some() {
            let history = match registry.history(&owner) {
                Ok(history) => history,
                Err(message) => {
                    found.push(problem(Code::U7010, message));
                    continue;
                }
            };
            let in_effect_at = |i: u64| history.took_effect.get(&i).copied();
            for (line, effect) in [(accepted_line, false), (effect_line, true)] {
                let Some(line) = line else {
                    continue;
                };
                for index in &line.indices {
                    let i = index.value;
                    let request = i < old_size && history.requests.contains_key(&i);
                    // A pending one not cancelled before the old point, by a cancellation that
                    // stands.
                    let cancelled = history
                        .cancellations
                        .get(&i)
                        .is_some_and(|cancels| cancels.iter().any(|c| *c < old_size));
                    let valid = if effect {
                        request && in_effect_at(i).is_some_and(|at| at <= old_size)
                    } else {
                        request && !cancelled && in_effect_at(i).is_none_or(|at| at > old_size)
                    };
                    if valid {
                        if effect {
                            recorded_effects.insert(i);
                        } else {
                            recorded.insert(i);
                        }
                    } else if reported.insert((identity.source.clone(), owner.clone(), i)) {
                        lock_errors.push(ProjectError {
                            code: Code::U7009,
                            range: index.range.clone(),
                            message: format!(
                                "the {} index {i} of `{owner}` is not a recovery, a dispute or a co-signer change of its chain {} at the lock's resolution point; run `una ita lock`",
                                if effect { "effect" } else { "accepted" },
                                if effect { "in effect" } else { "pending" }
                            ),
                        });
                    }
                }
            }
        }
        let ceiling = need.map(|need| canonical_ceiling(&need.may)).unwrap_or_default();
        if !ceiling.is_empty() {
            block.push_str(&format!("  granted by {root} may {ceiling}\n"));
        }
        if let Some(need) = need {
            found.extend(ceiling_consent(
                need,
                existing_lock,
                &root,
                options.accept_authority,
                repair,
            ));
        }
        // The release the lock it replaces holds for the identity, if any.
        let old = existing_lock.and_then(|lock| {
            lock.read.packages.iter().find(|p| {
                p.name.value.to_string() == identity.name
                    && matches!(
                        p.from.as_ref().map(|f| &f.value),
                        Some(ita::LockOrigin::Source(s)) if *s == identity.source
                    )
                    && Version::parse_full(&p.version.value)
                        .is_ok_and(|v| v.line() == identity.line)
            })
        });
        if let Some(old) = old {
            // Rule 26, from the two statements: never more authority or trust in one identity.
            if let Ok(old_version) = Version::parse_full(&old.version.value)
                && old_version != *version
            {
                match registry.release(&identity.name, &old_version) {
                    Ok(old_release) => {
                        if let Err(why) = una_project::tlog::release::within(
                            &release.statement,
                            &old_release.statement,
                        ) {
                            found.push(problem(
                                Code::U7010,
                                format!("{why}: the registry logged a release that breaks rule 26"),
                            ));
                            continue;
                        }
                    }
                    Err(message) => {
                        found.push(problem(Code::U7010, message));
                        continue;
                    }
                }
            }
            // A key change of the owner (rule 25), consented to only with `--accept-key`.
            let old_signed = old.signed.as_ref().map(|s| s.value.as_str());
            let old_point = locked_point(&identity.source).map(|(size, _)| size);
            match crate::registry::key_changed(
                &mut registry,
                &identity.name,
                old_signed,
                old_point,
                &signed,
                &recorded,
                &recorded_effects,
            ) {
                Ok(true) if !options.accept_key.contains(&identity.name) => {
                    found.push(problem(
                        Code::U7014,
                        format!(
                            "the keys of the owner of `{}` changed since ita.lock was written (now `{signed}`); review its key events in the registry's log, then run `una ita {repair} --accept-key {}`",
                            identity.name, identity.name
                        ),
                    ));
                    continue;
                }
                Ok(_) => {}
                Err(message) => {
                    found.push(problem(Code::U7010, message));
                    continue;
                }
            }
        }
        // A pending recovery, dispute or co-signer change of the owner (rule 38.1): reported,
        // and consented to only with `--accept-key`, as a key change is; the consent is
        // recorded in the lock, so a later lock, and `--check`, take it as given until a new
        // entry is pending.
        let flagged = options.accept_key.contains(&identity.name);
        match registry.history(una_project::client::owner_of(&identity.name)) {
            Ok(history)
                if history
                    .pending
                    .iter()
                    .any(|p| !flagged && !recorded.contains(&p.index)) =>
            {
                let pending: Vec<String> = history
                    .pending
                    .iter()
                    .filter(|p| !recorded.contains(&p.index))
                    .map(pending_change)
                    .collect();
                found.push(problem(
                    Code::U7014,
                    format!(
                        "the owner of `{}` has {} pending: {}; review it in the registry's log, then run `una ita {repair} --accept-key {}`",
                        identity.name,
                        if pending.len() == 1 { "a key change" } else { "key changes" },
                        pending.join("; "),
                        identity.name
                    ),
                ));
                continue;
            }
            Ok(history) => {
                // The effects consented to: those recorded, and, with `--accept-key`, those
                // that took effect since the lock it replaces.
                let mut effects = recorded_effects.clone();
                if flagged && old_registry.is_some() {
                    effects.extend(
                        history
                            .took_effect
                            .iter()
                            .filter(|(_, from)| **from > old_size)
                            .map(|(by, _)| *by),
                    );
                }
                if !effects.is_empty() {
                    effects_out
                        .entry(identity.source.clone())
                        .or_default()
                        .entry(owner.clone())
                        .or_default()
                        .extend(effects);
                }
                if !history.pending.is_empty() {
                    accepted_out
                        .entry(identity.source.clone())
                        .or_default()
                        .entry(owner.clone())
                        .or_default()
                        .extend(history.pending.iter().map(|p| p.index));
                }
            }
            Err(message) => {
                found.push(problem(Code::U7010, message));
                continue;
            }
        }
        // Trust (rules 10, 25 and 53), from the statement, which its archive is checked against
        // when it is fetched (rule 37).
        if !release.statement.trust.is_empty() {
            let trusted: Vec<&str> = release.statement.trust.iter().map(String::as_str).collect();
            let line = read.trust.iter().find(|line| {
                line.name.value.to_string() == identity.name
                    && match &line.from {
                        Some(from) => from.value == identity.source,
                        None => need.is_some(),
                    }
                    && line.major.as_ref().is_none_or(|major| {
                        una_project::version::Line::parse(&major.value) == Some(identity.line)
                    })
            });
            let covered = line.is_some_and(|line| {
                trusted
                    .iter()
                    .all(|t| line.modules.iter().any(|m| m.value == *t))
            });
            let digest = &release.statement.digest;
            let consented = old.is_some_and(|p| {
                p.consented.as_ref().is_some_and(|c| c.value == *digest)
                    && trusted
                        .iter()
                        .all(|t| p.trusted.iter().any(|m| m.value == *t))
            });
            if !covered {
                found.push(problem(
                    Code::U7012,
                    format!(
                        "`{}` has trusted modules, which no trust line of the root names; write `trust` with `{} {} from {} modules {}` after reviewing them",
                        identity.name,
                        identity.name,
                        identity.line,
                        identity.source,
                        trusted.join(", ")
                    ),
                ));
                continue;
            }
            if !consented && !options.accept_trust.contains(&identity.name) {
                found.push(problem(
                    Code::U7012,
                    format!(
                        "the trust of `{}` is not consented for its content as it is now; review its trusted modules and the code they run, then run `una ita {repair} --accept-trust {}`",
                        identity.name, identity.name
                    ),
                ));
                continue;
            }
            block.push_str(&format!(
                "  consented \"{digest}\"\n  trusted modules {}\n",
                trusted.join(", ")
            ));
        }
        used.insert(identity.source.clone());
        entries.push(Locked {
            name: identity.name.clone(),
            version: version.clone(),
            origin: identity.source.clone(),
            digest: release.statement.digest.clone(),
            ceiling,
            block,
            authority: Some(crate::registry::authority_of(&release)),
        });
    }
    diagnostics.extend(located(sources, manifest_id, found));
    if let Some((lock_id, _, _)) = &existing {
        diagnostics.extend(located(sources, *lock_id, lock_errors));
    }
    if !diagnostics.is_empty() {
        return finish_resolved(command, &[], &[], &[], &diagnostics, sources, out);
    }
    entries.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    let mut text = write_lock(&solver);
    for source in &used {
        let (Some(cell), Some(declared)) = (
            online.registries.get(source),
            crate::registry::declared(read, source),
        ) else {
            continue;
        };
        let (Ok(registry), Ok(pin)) = (cell.try_borrow(), crate::registry::pin_of(declared))
        else {
            continue;
        };
        let accepted = accepted_out.get(source).cloned().unwrap_or_default();
        let effects = effects_out.get(source).cloned().unwrap_or_default();
        text.push_str(&crate::registry::registry_block(
            source, pin, &registry, &accepted, &effects,
        ));
    }
    for entry in &entries {
        text.push_str(&entry.block);
    }
    if options.check {
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
        return finish_resolved(command, &[], &[], &[], &diagnostics, sources, out);
    }
    let unchanged = edited.is_none()
        && existing
            .as_ref()
            .is_some_and(|(id, _, _)| sources.file(*id).text() == text);
    if unchanged {
        notes.push("ita.lock is up to date".to_owned());
        return finish_resolved(command, &[], &[], &notes, &diagnostics, sources, out);
    }
    let changes = review(&package, existing_lock, &entries);
    let mut lines: Vec<String> = notes;
    lines.extend(changes.iter().map(ChangeDoc::line));
    if options.dry_run {
        lines.push("--dry-run: nothing written".to_owned());
        return finish_resolved(command, &[], &changes, &lines, &diagnostics, sources, out);
    }
    let mut written: Vec<&str> = Vec::new();
    if let Some(edited) = &edited {
        if let Err(error) = crate::write_through_rename(&manifest_path, edited) {
            writeln!(
                out.stderr,
                "error: cannot write `{}`: {error}",
                manifest_path.display()
            )?;
            return Ok(ExitStatus::Usage);
        }
        written.push("ita.una");
        lines.push("wrote ita.una".to_owned());
    }
    let lock_unchanged = existing
        .as_ref()
        .is_some_and(|(id, _, _)| sources.file(*id).text() == text);
    if !lock_unchanged {
        if let Err(error) = crate::write_through_rename_or_create(&lock_path, &text) {
            writeln!(
                out.stderr,
                "error: cannot write `{}`: {error}",
                lock_path.display()
            )?;
            return Ok(ExitStatus::Usage);
        }
        written.push("ita.lock");
        lines.push("wrote ita.lock".to_owned());
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

/// The consent of rule 25 for the ceiling of `need`: each operation its `may` lines allow that
/// the lock it replaces does not record as granted by the root is U7009 at the `may` line,
/// unless `accept` (`--accept-authority`). With no lock yet, a non-empty ceiling is not
/// recorded either.
fn ceiling_consent(
    need: &ita::Need,
    existing: Option<&una_project::ita::ItaLock>,
    root: &str,
    accept: bool,
    repair: &str,
) -> Vec<ProjectError> {
    if accept {
        return Vec::new();
    }
    let Some(lock) = existing else {
        if need.may.is_empty() {
            return Vec::new();
        }
        return vec![ProjectError {
            code: Code::U7009,
            range: need.range.clone(),
            message: format!(
                "the ceiling of `{}` is not recorded yet; review what it grants and run `una ita {repair} --accept-authority`",
                need.name.value
            ),
        }];
    };
    let recorded = recorded_ceiling(lock, &need.name.value, root);
    let mut problems = Vec::new();
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
            problems.push(ProjectError {
                code: Code::U7009,
                range: cap.range.clone(),
                message: format!(
                    "the ceiling of `{}` widens to `{}[{}]`; review what it grants and run `una ita {repair} --accept-authority`",
                    need.name.value,
                    cap.capability,
                    wider.join(", ")
                ),
            });
        }
    }
    problems
}

/// The review of rule 25: each package the new lock adds, changes or drops against the lock it
/// replaces, compared by name and origin. What a path package's code uses comes from the new
/// graph, checked at tier 0, only when something changed; a registry release's, from its
/// statement, which its archive is checked against when it is fetched (rule 37). O(p log p) for p packages, plus the check.
fn review(
    package: &crate::package::Package,
    existing: Option<&una_project::ita::ItaLock>,
    entries: &[Locked],
) -> Vec<ChangeDoc> {
    struct Old {
        version: String,
        digest: Option<String>,
        ceiling: String,
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
        old.insert(
            (locked.name.value.to_string(), origin),
            Old {
                version: locked.version.value.clone(),
                digest: locked.digest.as_ref().map(|d| d.value.clone()),
                ceiling,
            },
        );
    }
    let mut changes: Vec<ChangeDoc> = Vec::new();
    for entry in entries {
        let origin = entry.origin.clone();
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
                authority: entry.authority.clone().unwrap_or_default(),
            },
            Some(before) => {
                let same = before.version == version
                    && before.digest.as_deref() == Some(entry.digest.as_str())
                    && before.ceiling == entry.ceiling;
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
                    authority: entry.authority.clone().unwrap_or_default(),
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
        for change in changes
            .iter_mut()
            .filter(|c| c.change != "drop" && c.origin.starts_with("path "))
        {
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
    let status = if diagnostics.is_empty() {
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

/// `una ita remove <name> [path]` (ADR 0122 rule 57): removes the need of `name` from
/// `ita.una`, its indented lines with it, and the `needs` section when it empties, through a
/// temporary file and a rename; then writes the lock as `una ita lock` does. A name the root
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
    let Some(section) = lines.lines.iter().find(|line| {
        line.tokens
            .first()
            .is_some_and(|t| t.item == ita::Item::Word("needs".to_owned()))
    }) else {
        return Ok(ExitStatus::Usage);
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
            return Ok(ExitStatus::Usage);
        };
        (line_start(child.range.start), line_end(last_of(child)))
    };
    let mut edited = String::with_capacity(text.len());
    edited.push_str(text.get(..from).unwrap_or(""));
    edited.push_str(text.get(to..).unwrap_or(""));
    if let Err(error) = crate::write_through_rename(&manifest_path, &edited) {
        writeln!(
            out.stderr,
            "error: cannot write `{}`: {error}",
            manifest_path.display()
        )?;
        return Ok(ExitStatus::Usage);
    }
    lock_command(
        path,
        LockOptions {
            command: "remove",
            ..LockOptions::lock(false, false, false, &[])
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
/// (ADR 0122 rules 25 and 57): raises the named needs, or all, and writes the lock. A name the
/// root does not need, or an `ita.una` that does not parse, is a usage error. A path package is
/// always locked as its directory holds it, so `update` locks it again with the review of rule
/// 25; a registry need is raised in `ita.una` to the newest release of its identity that is not
/// a pre-release (unless the need names one), not yanked, avoided or signed after its key's
/// stated compromise, and logged at least the cooldown before the resolution point. It never
/// changes a major line.
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
    resolve_and_lock(
        path,
        LockOptions {
            raise: Some(names),
            ..options
        },
        out,
    )
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

/// The newest release of `name` at `source` that `una ita add` takes with no version (rule 25):
/// of the newest major line, not a pre-release, not signed after its key's stated compromise,
/// and logged at least the cooldown before a fresh resolution point, read from the registry
/// and verified, from the first of its places whose copy verifies. `itaty`, a source with no
/// place this release can read, and a registry that fails verification are refused, as
/// `una ita lock` refuses them.
fn newest_release(
    read: &ita::Manifest,
    lock_floor: Option<&(u64, String)>,
    source: &str,
    name: &str,
    options: &LockOptions<'_>,
) -> Result<Version, ProjectError> {
    use una_project::client::Point;

    let refuse = |code: Code, message: String| ProjectError {
        code,
        range: 0..0,
        message,
    };
    if source == una_project::ita::ITATY {
        return Err(refuse(
            Code::U7013,
            format!(
                "the newest release of `{name}` cannot be found, since the root metadata of `itaty` is not built into this release, so its per-name state cannot be verified"
            ),
        ));
    }
    let mirrors = crate::registry::load_mirrors(options.itaoca).map_err(|errors| {
        errors
            .into_iter()
            .next()
            .unwrap_or_else(|| {
                refuse(
                    Code::U7008,
                    "Itaoca's mirrors.una does not read".to_owned(),
                )
            })
    })?;
    let declared = crate::registry::declared(read, source).ok_or_else(|| {
        refuse(
            Code::U7013,
            format!("`{source}` is not a source of `sources`"),
        )
    })?;
    let places = crate::registry::places_of(declared, &mirrors);
    let (mut registry, _) = crate::registry::open_first(
        declared,
        &places,
        &Point::Latest { now: options.now },
        options.itaoca,
        lock_floor,
    )
    .map_err(|(code, message)| refuse(code, message))?;
    let releases = registry
        .releases(name)
        .map_err(|message| refuse(Code::U7010, message))?;
    let now = registry.point.time;
    let mut newest: Option<Version> = None;
    let mut held: Option<Version> = None;
    for release in &releases {
        let version = &release.statement.version;
        if release.compromised || version.is_pre() {
            continue;
        }
        let logged = registry
            .logged_at(release.index)
            .map_err(|message| refuse(Code::U7010, message))?;
        let candidate = if logged.saturating_add(options.cooldown) <= now {
            &mut newest
        } else {
            &mut held
        };
        if candidate.as_ref().is_none_or(|best| version > best) {
            *candidate = Some(version.clone());
        }
    }
    match (newest, held) {
        (Some(newest), _) => Ok(newest),
        (None, Some(held)) => Err(refuse(
            Code::U7013,
            format!(
                "every release of `{name}` at `{source}` is within the cooldown, the newest {held}; `--now` takes it"
            ),
        )),
        (None, None) => Err(refuse(
            Code::U7013,
            format!("`{source}` has no release of `{name}`"),
        )),
    }
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
    // Rule 42: a name whose owner a source of `sources` serves is needed from that source, and
    // `add` writes the `from`.
    let owner = name.split_once('/').map(|(owner, _)| owner);
    let serving = owner.and_then(|owner| {
        before
            .sources
            .iter()
            .find(|source| source.owners.iter().any(|o| o.value == owner))
            .map(|source| source.name.value.clone())
    });
    let from: Option<String> = from.map(str::to_owned).or(serving);
    let version: String = match version {
        Some(version) => version.to_owned(),
        None => {
            // Rule 25: the newest release of the newest major line past the cooldown, from the
            // registry's verified state.
            let source = from.clone().unwrap_or_else(|| una_project::ita::ITATY.to_owned());
            // The lock's resolution point of the source, which the new point must extend.
            let lock_floor = std::fs::read_to_string(path.join("ita.lock"))
                .ok()
                .and_then(|text| {
                    parse_ita_lock(&text)
                        .0
                        .read
                        .registries
                        .iter()
                        .find(|r| r.source.value == source)
                        .map(|r| (r.checkpoint_size.value, r.checkpoint_hash.value.clone()))
                });
            match newest_release(&before, lock_floor.as_ref(), &source, name, &options) {
                Ok(version) => version.to_string(),
                Err(error) => {
                    let mut sources = SourceDb::new();
                    let id = sources.add("ita.una".to_owned(), text.as_bytes());
                    let diagnostics = located(&sources, id, vec![error]);
                    return finish_resolved("add", &[], &[], &[], &diagnostics, &sources, out);
                }
            }
        }
    };
    let mut line = format!("  {name} {version}");
    if let Some(local) = local {
        line.push_str(&format!(" as {local}"));
    }
    if let Some(from) = &from {
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
/// stored beside it (rule 17): its files read again with no link followed, their digest, and
/// the stored statement, checkpoint and root. A copy that does not verify is U7010, a package
/// in neither place U7011; nothing is written or repaired. Path packages are the root author's
/// working tree, which `una ita lock --check` compares. No network. O(bytes of the packages).
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
        return Ok(ExitStatus::Usage);
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
        let what = format!("{} {}", locked.name.value, locked.version.value);
        let mut refuse = |code: Code, message: String| {
            problems.push(ProjectError {
                code,
                range: locked.range.clone(),
                message,
            });
        };
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
        if places.is_empty() && !failed {
            refuse(
                Code::U7011,
                format!(
                    "the locked package `{what}` is in neither the vendor directory nor Itaoca"
                ),
            );
        } else if !places.is_empty() {
            lines.push(format!("verified {what} in {}", places.join(" and ")));
        }
    }
    diagnostics.extend(located(sources, lock_id, problems));
    if lines.is_empty() && diagnostics.is_empty() {
        lines.push("no registry package is locked".to_owned());
    }
    finish("verify", &[], &lines, &diagnostics, sources, out)
}

/// `una ita fetch [path]` (ADR 0122 rules 30, 37, 39 and 40): fills Itaoca with each registry
/// release of `ita.lock` that it does not hold yet, read at the lock's resolution point from the
/// first place of its source, its mirrors then its own URL, whose copy verifies, and verified
/// as the registry client verifies
/// everything (`spec/30 §14`): the root chain from the pin of `sources`, the checkpoint the lock
/// records with its witnesses, the release statement's inclusion and its signature by the key
/// the lock names, valid at its log position, and the archive's digest; then the statement is
/// checked against the archive (rule 37), what its code uses included, the package is stored
/// with its provenance, and loaded again as every command loads it. No freshness is needed:
/// the lock's digests and the stored provenance decide. A release that fails is U7010, a source
/// with no place this release can read U7013, and nothing of it is stored. Path packages are not fetched. No program
/// is started. O(bytes of the archives) and the registry client's costs.
pub(crate) fn fetch_command(
    path: &Path,
    itaoca: Option<&una_project::itaoca::Itaoca>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    use una_project::client::{Point, Registry};
    use una_project::vendor;

    if !path.join("ita.una").is_file() {
        writeln!(
            out.stderr,
            "error: `{}` has no ita.una; `una ita` works on a package of edition 0.4, and `una migrate` moves a package to it",
            path.display()
        )?;
        return Ok(ExitStatus::Usage);
    }
    let Some(itaoca) = itaoca else {
        writeln!(
            out.stderr,
            "error: there is no Itaoca to fetch into: set `HOME`, or `UNA_ITAOCA` to a directory"
        )?;
        return Ok(ExitStatus::Usage);
    };
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
        diagnostics.extend(located(sources, manifest_id, vec![missing_lock()]));
        return finish("fetch", &[], &[], &diagnostics, sources, out);
    };
    let (lock, errors) = parse_ita_lock(sources.file(lock_id).text());
    diagnostics.extend(located(sources, lock_id, errors));
    let mirrors = match crate::registry::load_mirrors(Some(itaoca)) {
        Ok(mirrors) => mirrors,
        Err(errors) => {
            diagnostics.extend(located(sources, manifest_id, errors));
            una_project::mirrors::Mirrors::default()
        }
    };
    let Some(read) = manifest.ita.as_deref() else {
        return finish("fetch", &[], &[], &diagnostics, sources, out);
    };
    if !diagnostics.is_empty() {
        return finish("fetch", &[], &[], &diagnostics, sources, out);
    }
    let mut lines: Vec<String> = Vec::new();
    let mut problems: Vec<ProjectError> = Vec::new();
    // What the lock says of each registry release, and the places of each source, first: the
    // registries opened below borrow their transports.
    let mut wanted: Vec<(&ita::LockPackage, vendor::Expected)> = Vec::new();
    let mut places: BTreeMap<String, Vec<crate::registry::Place>> = BTreeMap::new();
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
        let mut refuse = |code: Code, message: String| {
            problems.push(ProjectError {
                code,
                range: locked.range.clone(),
                message,
            });
        };
        let expected = match vendor::expected(&lock.read, Some(read), locked) {
            Ok(expected) => expected,
            Err(message) => {
                refuse(Code::U7010, message);
                continue;
            }
        };
        match vendor::stored(itaoca, &expected) {
            Ok(Some(_)) => {
                lines.push(format!(
                    "{} {} is in Itaoca",
                    expected.name, expected.version
                ));
                continue;
            }
            Ok(None) => {}
            Err(vendor::Problem::Refused(message) | vendor::Problem::Absent(message)) => {
                refuse(Code::U7010, message);
                continue;
            }
        }
        if !places.contains_key(&expected.source) {
            match crate::registry::declared(read, &expected.source) {
                Some(declared) => {
                    places.insert(
                        expected.source.clone(),
                        crate::registry::places_of(declared, &mirrors),
                    );
                }
                None => {
                    refuse(
                        Code::U7013,
                        format!("`{}` is not a source of `sources`", expected.source),
                    );
                    continue;
                }
            }
        }
        wanted.push((locked, expected));
    }
    let mut registries: BTreeMap<String, Registry<'_>> = BTreeMap::new();
    for (locked, expected) in &wanted {
        let mut refuse = |code: Code, message: String| {
            problems.push(ProjectError {
                code,
                range: locked.range.clone(),
                message,
            });
        };
        let Some(list) = places.get(&expected.source) else {
            continue;
        };
        if !registries.contains_key(&expected.source) {
            let Some(declared) = crate::registry::declared(read, &expected.source) else {
                continue;
            };
            let point = Point::At {
                size: expected.checkpoint_size,
                hash: expected.checkpoint_hash.clone(),
            };
            match crate::registry::open_first(declared, list, &point, Some(itaoca), None) {
                Ok((registry, skipped)) => {
                    lines.extend(skipped);
                    registries.insert(expected.source.clone(), registry);
                }
                Err((code, message)) => {
                    refuse(code, message);
                    continue;
                }
            }
        }
        let Some(registry) = registries.get_mut(&expected.source) else {
            continue;
        };
        match fetch_one(registry, itaoca, expected) {
            Ok(()) => lines.push(format!(
                "fetched {} {}",
                expected.name, expected.version
            )),
            Err(message) => refuse(Code::U7010, message),
        }
    }
    // Each registry's floors were stored when its point was accepted (`registry::open`).
    diagnostics.extend(located(sources, lock_id, problems));
    if lines.is_empty() && diagnostics.is_empty() {
        lines.push("no registry package is locked".to_owned());
    }
    finish("fetch", &[], &lines, &diagnostics, sources, out)
}

/// Fetches one locked release into Itaoca, verified as [`fetch_command`] states.
fn fetch_one(
    registry: &mut una_project::client::Registry<'_>,
    itaoca: &una_project::itaoca::Itaoca,
    expected: &una_project::vendor::Expected,
) -> Result<(), String> {
    use una_project::provenance;
    use una_project::tlog::release::{self, check_archive};
    use una_project::tree::VerifiedTree;

    let what = format!("{} {}", expected.name, expected.version);
    let version = Version::parse_full(&expected.version)?;
    let release = registry.release(&expected.name, &version)?;
    if release.compromised {
        return Err(format!(
            "{what} counts as signed only with a key of its owner revoked as compromised before it was logged, so it is not fetched"
        ));
    }
    if release.index != expected.logged {
        return Err(format!(
            "the release statement of {what} is logged at {}, not at the lock's {}",
            release.index, expected.logged
        ));
    }
    if release.statement.digest != expected.digest {
        return Err(format!(
            "the release statement of {what} gives the digest {}, not the lock's {}",
            release.statement.digest, expected.digest
        ));
    }
    let key = release
        .signers
        .iter()
        .find(|key| provenance::signed_name(&expected.name, &key.public) == expected.signed)
        .ok_or_else(|| {
            format!(
                "{what} is not signed by `{}`, the key the lock names",
                expected.signed
            )
        })?
        .clone();
    let bytes = registry.archive(&expected.digest)?;
    let tree = VerifiedTree::from_archive(&bytes)?;
    // Rule 37: the archive's `ita.una`, needs and trust, and what its code uses, agree with the
    // statement. The code is checked at tier 0 from the verified bytes.
    check_archive(&release.statement, &tree, None)?;
    let used = archive_authority(&tree)?;
    let mut computed: release::Authority = BTreeMap::new();
    for (capability, op) in used {
        computed.entry(capability).or_default().insert(op);
    }
    check_archive(&release.statement, &tree, Some(&computed))?;
    let document = registry.provenance(&release, &key)?;
    provenance::verify(&document, expected)?;
    let stored = itaoca.store(&bytes)?;
    if stored != expected.digest {
        return Err(format!(
            "the archive of {what} stored as {stored}, not the lock's {}",
            expected.digest
        ));
    }
    itaoca.store_provenance(
        &expected.source,
        &expected.name,
        &expected.version,
        expected.checkpoint_size,
        &document.encode(),
    )?;
    // Loaded again as every command loads it.
    match una_project::vendor::stored(itaoca, expected) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(format!("{what} is not in Itaoca after it was stored")),
        Err(
            una_project::vendor::Problem::Refused(message)
            | una_project::vendor::Problem::Absent(message),
        ) => Err(message),
    }
}

/// What the code of a verified archive uses, checked at tier 0 from the archive's own bytes
/// (ADR 0122 rule 37): the package is read from the verified bytes in memory, so no byte of it
/// is read from a file that could have changed since it was verified. Its own needs are not
/// loaded, so the only diagnostics it may have are those that says: the lock an archive never
/// holds (U7009), its registry needs, which a package read alone cannot load or name the source
/// of (U7008 at its `ita.una`), and imports of their modules (U1001). Any other diagnostic, a module that does
/// not parse or check, or a platform the checker cannot run on, is refused: what such code
/// uses cannot be counted, and a count that is short would let a statement understate it.
fn archive_authority(
    tree: &una_project::tree::VerifiedTree,
) -> Result<una_project::authority::Authority, String> {
    let files: Vec<(String, String)> = tree
        .files()
        .iter()
        .map(|file| {
            String::from_utf8(file.bytes.clone())
                .map(|text| (file.path.clone(), text))
                .map_err(|_| format!("`{}` of the archive is not UTF-8", file.path))
        })
        .collect::<Result<_, _>>()?;
    let package = crate::package::load_files(&files).map_err(|error| error.to_string())?;
    // The places where a diagnostic is expected, by position, never by its text: each need's
    // line of `ita.una`, and the module path of each import of a module of a need.
    let mut need_lines: Vec<una_diag::Location> = Vec::new();
    let mut locals: Vec<String> = Vec::new();
    if let Some(id) = package.manifest {
        let (read, _) = una_syntax::ita::parse_manifest(package.sources.file(id).text());
        for need in &read.needs {
            need_lines.push(
                package
                    .sources
                    .location(Span::new(id, need.range.start, need.range.end)),
            );
            locals.push(una_project::ita::local_name(need).value);
        }
    }
    let mut import_paths: Vec<una_diag::Location> = Vec::new();
    for id in &package.modules {
        let text = package.sources.file(*id).text();
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            if let Some(rest) = line.strip_prefix("import ") {
                let path: String = rest
                    .chars()
                    .take_while(|c| !c.is_whitespace() && *c != '(')
                    .collect();
                let first = path.split('.').next().unwrap_or_default();
                if locals.iter().any(|local| local == first) {
                    let start = offset + "import ".len();
                    import_paths.push(
                        package
                            .sources
                            .location(Span::new(*id, start, start + path.len())),
                    );
                }
            }
            offset += line.len();
        }
    }
    let analysis = crate::on_large_stack(|| crate::analyze(&package, None));
    let within = |inner: &una_diag::Location, outer: &una_diag::Location| {
        inner.file == outer.file && inner.start >= outer.start && inner.end <= outer.end
    };
    let allowed = |diagnostic: &Diagnostic| match diagnostic.code {
        // No lock: an archive never holds one; reported at the start of `ita.una`.
        Code::U7009 => {
            diagnostic.primary.file == "ita.una"
                && diagnostic.primary.start.line == 1
                && diagnostic.primary.start.col == 1
        }
        // A need it cannot load alone, at the need's own line.
        Code::U7008 => need_lines
            .iter()
            .any(|line| within(&diagnostic.primary, line)),
        // An import of a module of a need, at the import's own module path.
        Code::U1001 => import_paths.contains(&diagnostic.primary),
        _ => false,
    };
    if let Some(diagnostic) = analysis.diagnostics.iter().find(|d| !allowed(d)) {
        return Err(format!(
            "the archive's code does not check ({} at {}: {}), so what it uses cannot be counted",
            diagnostic.code,
            una_project::client::shown(&diagnostic.primary.file),
            una_project::client::shown(&diagnostic.message)
        ));
    }
    Ok(crate::report::authority_under(&analysis, ""))
}

/// A pending key change of an owner's chain as U7014 names it: what it is, where it was
/// logged, and what it names, each name escaped (rule 38.1).
fn pending_change(pending: &una_project::tlog::owner::Pending) -> String {
    use una_project::client::shown;
    use una_project::tlog::owner::Change;
    let index = pending.index;
    match &pending.change {
        Change::Recovery(key) => format!(
            "a recovery logged at {index} naming the key `{}`",
            shown(&key.name)
        ),
        Change::Dispute(key) => format!(
            "a dispute logged at {index} naming the key `{}`",
            shown(&key.name)
        ),
        Change::Cosigners(Some((k, keys))) => {
            let names: Vec<String> = keys
                .iter()
                .map(|key| format!("`{}`", shown(&key.name)))
                .collect();
            format!(
                "a co-signer change logged at {index} designating {} with a threshold of {k}",
                names.join(", ")
            )
        }
        Change::Cosigners(None) => {
            format!("a co-signer change logged at {index} removing the recovery cosigners")
        }
    }
}
