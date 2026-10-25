//! `una ita publish`, `yank`, `audit`, `watch`, `bundle` and `unbundle` (ADR 0122 rules 33, 36,
//! 36.2 and 41 to 43; `spec/30 §15`).
//!
//! A submission goes to the source the head's `publish to` names, at the URL that source is
//! declared with, through a transport. This release has only the directory transport, and a
//! source's URL is `https://`, so `publish` and `yank` stop there with a diagnostic saying that
//! there is no https transport yet; plain `http://` is never used. The owner's keys come from
//! `~/.una/keys/` ([`keyfile::owner_keys`]). What a command needs to know of a registry (a name's
//! chain head, its releases, yanks and advisories, an owner's key events) comes from the verified
//! registry client ([`client`]), never from a transport's word; until that client joins, those
//! commands stop before sending anything, and `audit` reports every registry package as not
//! verified (U7010) rather than as clean. `bundle` and `unbundle` need no client: the lock's
//! digests and the provenance stored beside each package decide. No command here opens a
//! connection or starts a program.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use una_diag::{Code, Diagnostic};
use una_project::ProjectError;
use una_project::content::{self, File};
use una_project::ita::{ItaLock, parse_ita_lock, parse_ita_manifest};
use una_project::itaoca::Itaoca;
use una_project::provenance::Expected;
use una_project::client::{Point, Registry, owner_of};
use una_project::registry::keyfile::{self, SigningKey};
use una_project::registry::submit::NameState;
use una_project::registry::{audit, bundle, submit, watch};
use una_project::transport::{self, Location, Transport};
use una_project::tree::VerifiedTree;
use una_project::version::Version;
use una_project::{tree, vendor};
use una_syntax::{FileId, SourceDb, ita};

use crate::ExitStatus;
use crate::ita::{Output, finish, located};
use crate::verification::Solving;

/// A usage error: one line on stderr, exit code 2.
fn usage(out: Output<'_>, message: &str) -> io::Result<ExitStatus> {
    writeln!(out.stderr, "error: {message}")?;
    Ok(ExitStatus::Usage)
}

/// The usage error of a directory without `ita.una`.
fn not_a_package(path: &Path) -> String {
    format!(
        "`{}` has no ita.una; `una ita` works on a package of edition 0.4, and `una migrate` moves a package to it",
        path.display()
    )
}

/// A problem at `range` of a file.
fn problem(code: Code, range: std::ops::Range<usize>, message: String) -> ProjectError {
    ProjectError {
        code,
        range,
        message,
    }
}

/// The directory of the owner's key files, `.una/keys` in `HOME` (ADR 0122 rule 38).
pub(crate) fn keys_dir(home: Option<&str>) -> Option<PathBuf> {
    home.filter(|home| !home.is_empty())
        .map(|home| Path::new(home).join(".una").join("keys"))
}

/// What a submission needs, read verified from the source the head's `publish to` names.
struct Prepared {
    /// The transport of the source's own URL, which the submission is written to.
    target: Box<dyn Transport>,
    /// That URL.
    url: String,
    /// The name's state at a fresh checkpoint.
    state: NameState,
    /// The owner's keys that sign.
    keys: Vec<SigningKey>,
    /// The files of the highest earlier release of the version's line, for rule 26's advice.
    earlier: Option<Vec<File>>,
    /// Notes of mirrors skipped.
    notes: Vec<String>,
}

/// Reads what a submission of `name` to `source` needs (ADR 0122 rules 8, 38, 39, 40 and 43):
/// the source's declaration in the package's workspace, the transport of its own URL (an
/// `https://` one has none yet, U7013), the registry opened at a fresh checkpoint from its first
/// place that verifies (its mirrors, then its URL), the name's state, the owner's keys from
/// `keys`, and, with `version`, the files of the highest earlier release of its line. Each
/// problem is at `range`: U7010 for what does not verify, U7013 otherwise.
fn prepare(
    path: &Path,
    source: &str,
    name: &str,
    version: Option<&Version>,
    keys: Option<&Path>,
    itaoca: Option<&Itaoca>,
    range: std::ops::Range<usize>,
) -> Result<Prepared, ProjectError> {
    let at = |code: Code, why: String| problem(code, range.clone(), why);
    let declared = submit::declared_source(path, source).map_err(|why| {
        at(
            Code::U7013,
            format!("the submission cannot be sent to `{source}`: {why}; nothing is sent"),
        )
    })?;
    let url = declared.url.value.clone();
    let target = match Location::parse(&url) {
        Ok(Location::Https(_)) => {
            return Err(at(
                Code::U7013,
                format!(
                    "the submission cannot be sent to `{source}`: `{url}` is reached over https, and there is no https transport yet; nothing is sent"
                ),
            ));
        }
        Ok(location) => transport::open(&location),
        Err(why) => return Err(at(Code::U7013, why)),
    };
    let mirrors = crate::registry::load_mirrors(itaoca).map_err(|errors| {
        let why = errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
            .join("; ");
        at(Code::U7008, why)
    })?;
    let places = crate::registry::places_of(&declared, &mirrors);
    let point = Point::Latest {
        now: crate::local_clock(),
    };
    let (mut registry, notes) =
        crate::registry::open_first(&declared, &places, &point, itaoca, None)
            .map_err(|(code, why)| at(code, why))?;
    let state = NameState::read(&mut registry, name).map_err(|why| at(Code::U7010, why))?;
    let owner = owner_of(name);
    let history = registry
        .history(owner)
        .map_err(|why| at(Code::U7010, why))?;
    let snapshot = history
        .at(registry.point.checkpoint.size)
        .cloned()
        .ok_or_else(|| {
            at(
                Code::U7013,
                format!("the owner `{owner}` has no key at `{source}`; nothing is sent"),
            )
        })?;
    let Some(keys) = keys else {
        return Err(at(
            Code::U7013,
            "no key directory: set `HOME`, whose `.una/keys` holds the owner's key files"
                .to_owned(),
        ));
    };
    let keys = keyfile::owner_keys(keys, owner, &snapshot.keys, snapshot.threshold)
        .map_err(|why| at(Code::U7013, why))?;
    let earlier = match version {
        Some(version) => {
            let statement = state
                .statements
                .range(..version.clone())
                .rev()
                .find(|(v, _)| v.line() == version.line())
                .map(|(_, statement)| statement.clone());
            match statement {
                Some(statement) => {
                    let bytes = registry
                        .archive(&statement.digest)
                        .map_err(|why| at(Code::U7010, why))?;
                    let tree = VerifiedTree::from_archive(&bytes)
                        .map_err(|why| at(Code::U7010, why))?;
                    if tree.digest() != statement.digest {
                        return Err(at(
                            Code::U7010,
                            format!(
                                "the archive of `{name} {}` is not the one its statement names",
                                statement.version
                            ),
                        ));
                    }
                    Some(tree.into_files())
                }
                None => None,
            }
        }
        None => None,
    };
    if let Some(itaoca) = itaoca {
        // The checkpoint read is the source's floor from now on (rule 39).
        let _ = itaoca.store_checkpoint(source, &registry.point.document);
    }
    Ok(Prepared {
        target,
        url,
        state,
        keys,
        earlier,
        notes,
    })
}

/// The files a package publishes: its `ita.una` and modules, as checked, and its `README.md`,
/// `CHANGELOG.md` and license files at the top, each read once with no link followed. Every
/// other file of the directory, its lock included, stays out (rule 27).
fn package_files(root: &Path, package: &crate::package::Package) -> Result<Vec<File>, String> {
    let mut files = Vec::new();
    for id in package
        .manifest
        .into_iter()
        .chain(package.modules.iter().copied())
    {
        let file = package.sources.file(id);
        files.push(File {
            path: file.path().to_owned(),
            bytes: file.text().as_bytes().to_vec(),
        });
    }
    let mut names: Vec<String> = std::fs::read_dir(root)
        .map_err(|error| format!("cannot list `{}`: {error}", root.display()))?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name != "ita.una" && !name.contains('/') && content::is_package_file(name))
        .collect();
    names.sort();
    for name in names {
        let path = root.join(&name);
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot read `{name}`: {error}"))?;
        if !meta.is_file() {
            return Err(format!("`{name}` is not a regular file"));
        }
        files.push(File {
            path: name,
            bytes: tree::read_unlinked(&path)?,
        });
    }
    Ok(files)
}

/// The union of the `uses` of the package's public functions and of its handlers: capability
/// and operations (rule 37).
fn authority(
    analysis: &crate::Analysis,
) -> BTreeMap<String, std::collections::BTreeSet<String>> {
    let mut public = std::collections::BTreeSet::new();
    for module in &analysis.program.modules {
        for decl in &module.decls {
            match decl {
                una_core::Decl::Fun(fun) if fun.public => {
                    public.insert(format!("{}.{}", module.path, fun.name));
                }
                _ => {}
            }
        }
    }
    let mut out: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    for header in &analysis.headers {
        if !public.contains(&format!("{}.{}", header.module, header.name)) {
            continue;
        }
        for used in &header.uses {
            out.entry(used.capability.clone())
                .or_default()
                .extend(used.ops.iter().cloned());
        }
    }
    for module in &analysis.program.modules {
        for decl in &module.decls {
            if let una_core::Decl::Handler { uses, .. } = decl {
                for used in uses {
                    out.entry(used.capability.clone())
                        .or_default()
                        .extend(used.ops.iter().cloned());
                }
            }
        }
    }
    out
}

/// `una ita publish [path]` (ADR 0122 rule 43): refuses what rule 43 names, and a package in a
/// git repository while the check for uncommitted changes is not available; checks the package
/// at tier 1 and refuses any diagnostic; builds its archive (rule 27); resolves the source the
/// head's `publish to` names and its transport; writes the release statement against the
/// verified state of its name (rule 26 against the highest earlier release of its line); signs
/// it with the owner's keys of `~/.una/keys/`; and sends the archive and the statement. The
/// submission is recorded in Itaoca, so `una ita watch` can check the registry's promise for
/// it. Costs: the check, and O(B) for the B bytes of the package.
pub(crate) fn publish_command(
    path: &Path,
    keys: Option<&Path>,
    solving: &Solving,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    if !path.join("ita.una").is_file() {
        return usage(out, &not_a_package(path));
    }
    let package = match crate::package::load_in(path, &BTreeMap::new(), solving.itaoca.as_ref()) {
        Ok(package) => package,
        Err(error) => return usage(out, &error.to_string()),
    };
    let sources = &package.sources;
    let Some(manifest_id) = package.manifest else {
        return Ok(ExitStatus::Usage);
    };
    let (manifest, errors) = parse_ita_manifest(sources.file(manifest_id).text());
    let mut diagnostics = located(sources, manifest_id, errors);
    let Some(read) = manifest.ita.as_deref() else {
        return finish("publish", &[], &[], &diagnostics, sources, out);
    };
    if !diagnostics.is_empty() {
        return finish("publish", &[], &[], &diagnostics, sources, out);
    }
    let head = read
        .publish_to
        .as_ref()
        .map_or(read.head.clone(), |target| target.range.clone());
    let refusals: Vec<ProjectError> = una_project::tlog::publish::refusals(read)
        .into_iter()
        .map(|message| problem(Code::U7013, head.clone(), message))
        .collect();
    if !refusals.is_empty() {
        diagnostics.extend(located(sources, manifest_id, refusals));
        return finish("publish", &[], &[], &diagnostics, sources, out);
    }
    // Rule 43 refuses a workspace member with uncommitted changes; until that check is
    // available, a package in a git repository is refused, and no program is started to look.
    if let Some(git) = submit::git_entry(path) {
        diagnostics.extend(located(
            sources,
            manifest_id,
            vec![problem(
                Code::U7013,
                head,
                format!(
                    "the check for uncommitted changes is not available yet, so a package in a git repository is not published (found `{}`)",
                    git.display()
                ),
            )],
        ));
        return finish("publish", &[], &[], &diagnostics, sources, out);
    }
    // `una check` at tier 1: any diagnostic, an obligation not proven included, refuses.
    let analysis = crate::on_large_stack(|| crate::analyze(&package, None));
    let mut checked = analysis.diagnostics.clone();
    if checked.is_empty() {
        let found = crate::verification::tier_one(&package, &analysis.program, solving);
        checked.extend(found.diagnostics);
    }
    if !checked.is_empty() {
        let count = checked.len();
        diagnostics.extend(checked);
        diagnostics.extend(located(
            sources,
            manifest_id,
            vec![problem(
                Code::U7013,
                head,
                format!(
                    "`una check` of the package reports {count} diagnostics; a package is published only when it checks at tier 1 with none"
                ),
            )],
        ));
        return finish("publish", &[], &[], &diagnostics, sources, out);
    }
    let files = match package_files(path, &package) {
        Ok(files) => files,
        Err(message) => {
            diagnostics.extend(located(
                sources,
                manifest_id,
                vec![problem(Code::U7013, head, message)],
            ));
            return finish("publish", &[], &[], &diagnostics, sources, out);
        }
    };
    let source = read
        .publish_to
        .as_ref()
        .map(|t| t.value.clone())
        .unwrap_or_default();
    let name = read
        .name
        .as_ref()
        .map(|n| n.value.to_string())
        .unwrap_or_default();
    let version = read
        .version
        .as_ref()
        .and_then(|v| Version::parse_full(&v.value).ok());
    let prepared = match prepare(
        path,
        &source,
        &name,
        version.as_ref(),
        keys,
        solving.itaoca.as_ref(),
        head.clone(),
    ) {
        Ok(prepared) => prepared,
        Err(refusal) => {
            diagnostics.extend(located(sources, manifest_id, vec![refusal]));
            return finish("publish", &[], &[], &diagnostics, sources, out);
        }
    };
    // Rule 26's advice against the highest earlier release of the line: nothing is refused.
    let mut lines = prepared.notes.clone();
    if let Some(earlier) = &prepared.earlier {
        lines.extend(
            submit::header_advice(earlier, &files)
                .into_iter()
                .map(|advice| format!("advice: {advice}")),
        );
    }
    let (project, _) = crate::package::project(&package);
    let trust: std::collections::BTreeSet<String> =
        project.trusted.iter().map(|t| t.value.clone()).collect();
    let signers: Vec<&SigningKey> = prepared.keys.iter().collect();
    let submission = match submit::release(
        read,
        files,
        &authority(&analysis),
        &trust,
        &prepared.state,
        &signers,
    ) {
        Ok(submission) => submission,
        Err(refusals) => {
            diagnostics.extend(located(
                sources,
                manifest_id,
                refusals
                    .into_iter()
                    .map(|message| problem(Code::U7013, head.clone(), message))
                    .collect(),
            ));
            return finish("publish", &[], &[], &diagnostics, sources, out);
        }
    };
    lines.push(format!(
        "published {name} {} to `{}`",
        read.version.as_ref().map(|v| v.value.as_str()).unwrap_or_default(),
        prepared.url
    ));
    send_and_record(
        "publish",
        &submission,
        prepared.target.as_ref(),
        (&source, &prepared.url),
        solving.itaoca.as_ref(),
        lines,
        (sources, manifest_id, head),
        out,
    )
}

/// Sends a submission, records it in Itaoca for `una ita watch`, and prints what was done.
#[allow(clippy::too_many_arguments)]
fn send_and_record(
    command: &str,
    submission: &submit::Submission,
    transport: &dyn Transport,
    (source, url): (&str, &str),
    itaoca: Option<&Itaoca>,
    done: Vec<String>,
    (sources, file, range): (&SourceDb, FileId, std::ops::Range<usize>),
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    let written = match submit::send(transport, submission) {
        Ok(written) => written,
        Err(message) => {
            let diagnostics = located(sources, file, vec![problem(Code::U7013, range, message)]);
            return finish(command, &[], &[], &diagnostics, sources, out);
        }
    };
    let mut lines = done;
    lines.push(format!(
        "the log will hold the leaf {}; `una ita watch` checks the registry's promise to log it",
        submission.leaf
    ));
    if let Some(itaoca) = itaoca
        && let Err(why) = record_submission(itaoca, source, submission, url)
    {
        lines.push(format!("the submission is not recorded in Itaoca: {why}"));
    }
    let written: Vec<&str> = written.iter().map(String::as_str).collect();
    finish(command, &written, &lines, &[], sources, out)
}

/// Records a submission sent: `registry/<source>/submitted/<leaf hex>` in Itaoca holds the URL
/// it was sent to, where its promise appears.
fn record_submission(
    itaoca: &Itaoca,
    source: &str,
    submission: &submit::Submission,
    url: &str,
) -> Result<(), String> {
    vendor::check_source(source)?;
    let leaf = submission
        .leaf
        .strip_prefix("sha256:")
        .ok_or("a leaf hash is `sha256:` and hex")?;
    tree::not_a_link(&itaoca.root)?;
    let parent = tree::make_dirs(&itaoca.root, &["registry", source, "submitted"])?;
    tree::store_file(&parent, leaf, format!("{url}\n").as_bytes(), false)
}

/// `una ita yank <version> [path]` (ADR 0122 rules 38 and 41, and its amendment of the yank
/// command): writes a yank of a release of the package at `path`, against the verified state of
/// its name, signs it with the owner's keys of `~/.una/keys/`, at least the owner's threshold,
/// and sends it to the source the head's `publish to` names, through the same transport as
/// `publish`.
pub(crate) fn yank_command(
    version: &str,
    path: &Path,
    keys: Option<&Path>,
    itaoca: Option<&Itaoca>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    let manifest_path = path.join("ita.una");
    if !manifest_path.is_file() {
        return usage(out, &not_a_package(path));
    }
    let Ok(version) = Version::parse_full(version) else {
        return usage(out, &format!("`{version}` is not a full version, such as `0.3.1`"));
    };
    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => text,
        Err(error) => return usage(out, &format!("cannot read ita.una: {error}")),
    };
    let mut sources = SourceDb::new();
    let id = sources.add("ita.una".to_owned(), text.as_bytes());
    let (manifest, errors) = parse_ita_manifest(&text);
    let mut diagnostics = located(&sources, id, errors);
    let Some(read) = manifest.ita.as_deref().filter(|_| diagnostics.is_empty()) else {
        return finish("yank", &[], &[], &diagnostics, &sources, out);
    };
    let head = read.head.clone();
    let (Some(source), Some(name)) = (
        read.publish_to.as_ref().map(|t| t.value.clone()),
        read.name.as_ref().map(|n| n.value.to_string()),
    ) else {
        diagnostics.extend(located(
            &sources,
            id,
            vec![problem(
                Code::U7013,
                head,
                "the head has no `publish to <source>`, so the package has no registry to yank at"
                    .to_owned(),
            )],
        ));
        return finish("yank", &[], &[], &diagnostics, &sources, out);
    };
    let prepared = match prepare(path, &source, &name, None, keys, itaoca, head.clone()) {
        Ok(prepared) => prepared,
        Err(refusal) => {
            diagnostics.extend(located(&sources, id, vec![refusal]));
            return finish("yank", &[], &[], &diagnostics, &sources, out);
        }
    };
    let signers: Vec<&SigningKey> = prepared.keys.iter().collect();
    let submission = match submit::yank(&prepared.state, &version, &signers) {
        Ok(submission) => submission,
        Err(message) => {
            diagnostics.extend(located(
                &sources,
                id,
                vec![problem(Code::U7013, head, message)],
            ));
            return finish("yank", &[], &[], &diagnostics, &sources, out);
        }
    };
    let mut lines = prepared.notes.clone();
    lines.push(format!("yanked {name} {version} at `{}`", prepared.url));
    send_and_record(
        "yank",
        &submission,
        prepared.target.as_ref(),
        (&source, &prepared.url),
        itaoca,
        lines,
        (&sources, id, head),
        out,
    )
}

/// The package's `ita.una` and `ita.lock`, read for a command that works on the lock.
struct Project {
    package: crate::package::Package,
    manifest_id: FileId,
    lock_id: FileId,
    manifest: una_project::Manifest,
    lock: ItaLock,
}

/// Why a command on the lock ends before it starts: a usage error, or the diagnostics of the
/// package's files.
enum Ended {
    /// A usage error, exit code 2.
    Usage(String),
    /// The diagnostics of `ita.una` or `ita.lock`, in the files they were read from.
    Diagnostics(Vec<Diagnostic>, SourceDb),
}

impl Ended {
    /// Prints the end of `command`.
    fn finish(self, command: &str, out: Output<'_>) -> io::Result<ExitStatus> {
        match self {
            Ended::Usage(message) => usage(out, &message),
            Ended::Diagnostics(diagnostics, sources) => {
                finish(command, &[], &[], &diagnostics, &sources, out)
            }
        }
    }

    /// The codes it reports.
    fn codes(&self) -> Vec<Code> {
        match self {
            Ended::Usage(_) => Vec::new(),
            Ended::Diagnostics(diagnostics, _) => diagnostics.iter().map(|d| d.code).collect(),
        }
    }
}

/// Loads the package at `path` for a command on its lock: its `ita.una` and `ita.lock`, both
/// read without an error.
fn project(path: &Path) -> Result<Project, Ended> {
    if !path.join("ita.una").is_file() {
        return Err(Ended::Usage(not_a_package(path)));
    }
    let package = crate::package::load(path).map_err(|error| Ended::Usage(error.to_string()))?;
    let Some(manifest_id) = package.manifest else {
        return Err(Ended::Usage(not_a_package(path)));
    };
    let (manifest, errors) = parse_ita_manifest(package.sources.file(manifest_id).text());
    let mut diagnostics = located(&package.sources, manifest_id, errors);
    let Some(lock_id) = package.lock else {
        diagnostics.extend(located(
            &package.sources,
            manifest_id,
            vec![problem(
                Code::U7009,
                0..0,
                "the package has no ita.lock; run `una ita lock`".to_owned(),
            )],
        ));
        return Err(Ended::Diagnostics(diagnostics, package.sources));
    };
    let (lock, errors) = parse_ita_lock(package.sources.file(lock_id).text());
    diagnostics.extend(located(&package.sources, lock_id, errors));
    if !diagnostics.is_empty() {
        return Err(Ended::Diagnostics(diagnostics, package.sources));
    }
    Ok(Project {
        package,
        manifest_id,
        lock_id,
        manifest,
        lock,
    })
}

/// The source of a locked package, when it is a registry's.
fn registry_source(locked: &ita::LockPackage) -> Option<&str> {
    match &locked.from {
        Some(ita::At {
            value: ita::LockOrigin::Source(source),
            ..
        }) => Some(source.as_str()),
        _ => None,
    }
}

/// What the audit reads of each locked package: its facts, from the verified client.
type FactsOf<'f> = dyn FnMut(&audit::Locked) -> Result<audit::Facts, String> + 'f;

/// The diagnostics of `una ita audit` on `project`, each package's facts from `facts_of`, and
/// the lines it prints.
fn audit_project(project: &Project, facts_of: &mut FactsOf<'_>) -> (Vec<Diagnostic>, Vec<String>) {
    let sources = &project.package.sources;
    let mut problems: Vec<ProjectError> = Vec::new();
    let mut at_needs: Vec<ProjectError> = Vec::new();
    let mut packages: Vec<audit::Locked> = Vec::new();
    let mut lines_of: Vec<&ita::LockPackage> = Vec::new();
    for locked in &project.lock.read.packages {
        let Some(source) = registry_source(locked) else {
            continue;
        };
        let Ok(version) = Version::parse_full(&locked.version.value) else {
            continue;
        };
        let resolution = project
            .lock
            .read
            .registries
            .iter()
            .find(|r| r.source.value == source)
            .map_or(0, |r| r.checkpoint_size.value);
        packages.push(audit::Locked {
            source: source.to_owned(),
            name: locked.name.value.to_string(),
            version,
            logged: locked.logged.as_ref().map(|l| l.value),
            resolution,
        });
        lines_of.push(locked);
    }
    let mut lines = Vec::new();
    match audit::audit(&packages, facts_of) {
        Err((index, why)) => {
            if let (Some(locked), Some(package)) = (lines_of.get(index), packages.get(index)) {
                problems.push(problem(
                    Code::U7010,
                    locked.range.clone(),
                    format!(
                        "the state of `{}` at `{}` cannot be verified at a fresh checkpoint: {why}; the audit reports nothing as clean that it did not verify",
                        package.name, package.source
                    ),
                ));
            }
        }
        Ok(findings) => {
            let mut by_package: BTreeMap<usize, Vec<String>> = BTreeMap::new();
            for finding in &findings {
                if let Some(package) = packages.get(finding.package) {
                    by_package
                        .entry(finding.package)
                        .or_default()
                        .push(finding.text(package));
                }
            }
            for (index, texts) in by_package {
                let (Some(locked), Some(package)) = (lines_of.get(index), packages.get(index))
                else {
                    continue;
                };
                let message = format!(
                    "`{}` ({}): {}; run `una ita update {}`",
                    package.name,
                    package.source,
                    texts.join("; "),
                    package.name
                );
                // At the root's need when it needs the package directly, else at its lock
                // line.
                let need = project.manifest.ita.as_deref().and_then(|m| {
                    m.needs
                        .iter()
                        .find(|need| need.name.value.to_string() == package.name)
                });
                match need {
                    Some(need) => {
                        at_needs.push(problem(Code::U7014, need.range.clone(), message));
                    }
                    None => {
                        problems.push(problem(Code::U7014, locked.range.clone(), message));
                    }
                }
            }
            if packages.is_empty() {
                lines.push("no registry package is locked".to_owned());
            } else if findings.is_empty() {
                lines.push(format!(
                    "audited {} registry packages: none is yanked, advised, or of an owner whose keys changed",
                    packages.len()
                ));
            }
        }
    }
    let mut diagnostics = located(sources, project.manifest_id, at_needs);
    diagnostics.extend(located(sources, project.lock_id, problems));
    (diagnostics, lines)
}

/// `una ita audit [path]` (ADR 0122 rules 39 and 41, U7014): each registry package of the lock
/// against the verified state of its name and its owner, each source opened at a fresh
/// checkpoint from its first place that verifies (rule 40). Exit code 1 on any finding, and on
/// any state that cannot be verified (U7010).
pub(crate) fn audit_command(
    path: &Path,
    itaoca: Option<&Itaoca>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    let project = match project(path) {
        Ok(project) => project,
        Err(ended) => return ended.finish("audit", out),
    };
    let mirrors = match crate::registry::load_mirrors(itaoca) {
        Ok(mirrors) => mirrors,
        Err(errors) => {
            let diagnostics = located(&project.package.sources, project.manifest_id, errors);
            return finish("audit", &[], &[], &diagnostics, &project.package.sources, out);
        }
    };
    let read = project.manifest.ita.as_deref();
    let used: std::collections::BTreeSet<String> = project
        .lock
        .read
        .packages
        .iter()
        .filter_map(registry_source)
        .map(str::to_owned)
        .collect();
    let mut places: BTreeMap<String, (ita::Source, Vec<crate::registry::Place>)> =
        BTreeMap::new();
    for source in &used {
        if let Some(declared) = read.and_then(|read| crate::registry::declared(read, source)) {
            let list = crate::registry::places_of(declared, &mirrors);
            places.insert(source.clone(), (declared.clone(), list));
        }
    }
    let point = Point::Latest {
        now: crate::local_clock(),
    };
    let mut opened: BTreeMap<String, Result<Registry<'_>, String>> = BTreeMap::new();
    for (source, (declared, list)) in &places {
        let registry = crate::registry::open_first(declared, list, &point, itaoca, None)
            .map(|(registry, _)| registry)
            .map_err(|(_, why)| why);
        opened.insert(source.clone(), registry);
    }
    let mut facts_of = |locked: &audit::Locked| match opened.get_mut(&locked.source) {
        Some(Ok(registry)) => audit::facts(registry, locked),
        Some(Err(why)) => Err(why.clone()),
        None => Err(format!(
            "`{}` is not a source of ita.una's `sources`",
            locked.source
        )),
    };
    let (diagnostics, lines) = audit_project(&project, &mut facts_of);
    finish("audit", &[], &lines, &diagnostics, &project.package.sources, out)
}

/// The codes `una ita audit` reports for the package at `path`, each package's facts from
/// `facts_of`.
pub(crate) fn audit_codes(path: &Path, facts_of: &mut FactsOf<'_>) -> Vec<Code> {
    match project(path) {
        Ok(project) => audit_project(&project, facts_of)
            .0
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect(),
        Err(ended) => ended.codes(),
    }
}

/// The declaration of `source` for `una ita watch` in `dir`: the one `ita.una`'s `sources`
/// gives, else the one of the workspace that lists the package ([`submit::declared_source`]).
fn find_source(dir: &Path, source: &str) -> Result<ita::Source, String> {
    if let Ok(text) = std::fs::read_to_string(dir.join("ita.una")) {
        let (read, _) = ita::parse_manifest(&text);
        if let Some(declared) = read.sources.iter().find(|s| s.name.value == source) {
            return Ok(declared.clone());
        }
    }
    submit::declared_source(dir, source)
}

/// `una ita watch <owner> [<name>...] [--from <source>]` (ADR 0122 rules 36 and 36.2): prints
/// every entry of the owner's chain and of the chains of the names followed since the position
/// stored in Itaoca, each read verified at a fresh checkpoint, then stores the new position; and
/// checks the registry's promise for each submission recorded, reporting a broken one. The
/// source is the one the package in the current directory, or its workspace, declares. Exit
/// code 1 when a promise is broken or a chain does not verify.
pub(crate) fn watch_command(
    owner: &str,
    names: &[String],
    source: &str,
    itaoca: Option<&Itaoca>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    if let Err(message) = vendor::check_source(source) {
        return usage(out, &message);
    }
    if vendor::check_source(owner).is_err() && owner != "itauna" {
        return usage(out, &format!("`{owner}` is not an owner's name"));
    }
    let Some(itaoca) = itaoca else {
        return usage(
            out,
            "no Itaoca to keep the position in: set `UNA_ITAOCA` or `HOME`",
        );
    };
    let sources = SourceDb::new();
    let declared = match find_source(Path::new("."), source) {
        Ok(declared) => declared,
        Err(why) => return usage(out, &why),
    };
    let mirrors = match crate::registry::load_mirrors(Some(itaoca)) {
        Ok(mirrors) => mirrors,
        Err(errors) => {
            let why: Vec<String> = errors.into_iter().map(|e| e.message).collect();
            return usage(out, &why.join("; "));
        }
    };
    let places = crate::registry::places_of(&declared, &mirrors);
    let point = Point::Latest {
        now: crate::local_clock(),
    };
    let (mut registry, notes) =
        match crate::registry::open_first(&declared, &places, &point, Some(itaoca), None) {
            Ok(opened) => opened,
            Err((code, why)) => {
                writeln!(out.stderr, "error[{}]: {why}", code.as_str())?;
                return Ok(ExitStatus::Errors);
            }
        };
    let position_name = format!("{owner}.position");
    let stored = match tree::locate(&itaoca.root, &["registry", source, "watch", &position_name]) {
        Ok(Some(file)) => match tree::read_file(&file, 1 << 20)
            .and_then(|bytes| String::from_utf8(bytes).map_err(|_| "not UTF-8".to_owned()))
            .and_then(|text| watch::Position::parse(&text))
        {
            Ok(position) => position,
            Err(why) => {
                return usage(
                    out,
                    &format!("the stored position of `{owner}` does not read: {why}"),
                );
            }
        },
        Ok(None) => watch::Position::start(source, owner),
        Err(why) => return usage(out, &why),
    };
    let log = registry.root.log.clone();
    let promises = match recorded_promises(itaoca, source, &log) {
        Ok(promises) => promises,
        Err(why) => {
            writeln!(out.stderr, "error: {why}")?;
            return Ok(ExitStatus::Errors);
        }
    };
    let time = registry.point.time;
    let mut chain = |name: &str| watch::chain_of(&mut registry, name);
    let report = match watch::watch(&mut chain, time, &stored, names, &promises) {
        Ok(report) => report,
        Err(why) => {
            writeln!(
                out.stderr,
                "error[U7010]: the chains of `{owner}` at `{source}` cannot be followed: {why}"
            )?;
            return Ok(ExitStatus::Errors);
        }
    };
    let mut lines: Vec<String> = notes;
    lines.extend(report.entries.iter().map(|(name, entry)| {
        format!(
            "{name}: {} at log entry {} ({})",
            entry.format, entry.index, entry.leaf
        )
    }));
    for promise in &report.kept {
        lines.push(format!("kept: the promise to log {} by {}", promise.leaf, promise.by));
    }
    for promise in &report.broken {
        lines.push(format!(
            "broken: the registry promised to log {} by {}, and its checkpoint is later without it; publish the promise as evidence",
            promise.leaf, promise.by
        ));
    }
    if report.entries.is_empty() && report.kept.is_empty() && report.broken.is_empty() {
        lines.push(format!("nothing new for `{owner}` at `{source}`"));
    }
    let mut written: Vec<String> = Vec::new();
    let stored_position = tree::make_dirs(&itaoca.root, &["registry", source, "watch"])
        .and_then(|parent| {
            tree::store_file(
                &parent,
                &position_name,
                report.position.encode().as_bytes(),
                false,
            )
        });
    match stored_position {
        Ok(()) => written.push(format!("registry/{source}/watch/{position_name}")),
        Err(why) => lines.push(format!("the position is not stored: {why}")),
    }
    let written: Vec<&str> = written.iter().map(String::as_str).collect();
    let status = finish("watch", &written, &lines, &[], &sources, out)?;
    if report.broken.is_empty() {
        Ok(status)
    } else {
        Ok(ExitStatus::Errors)
    }
}

/// The promises of the submissions recorded in Itaoca, each read from the place it was sent to
/// and verified with the log's key `log`: a submission whose promise has not appeared yet is
/// skipped.
fn recorded_promises(
    itaoca: &Itaoca,
    source: &str,
    log: &una_project::tlog::note::Key,
) -> Result<Vec<submit::Promise>, String> {
    let Some(dir) = tree::locate(&itaoca.root, &["registry", source, "submitted"])? else {
        return Ok(Vec::new());
    };
    let mut leaves: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|error| format!("cannot list the submissions recorded: {error}"))?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect();
    leaves.sort();
    let mut promises = Vec::new();
    for leaf in leaves {
        let Some(file) = tree::locate(&dir, &[leaf.as_str()])? else {
            continue;
        };
        let url = String::from_utf8(tree::read_file(&file, 4096)?)
            .map_err(|_| "a recorded submission is not UTF-8".to_owned())?;
        let transport = transport::open(&Location::parse(url.trim_end())?);
        let Some(bytes) = transport.get(&format!("submit/{leaf}/promise"), 1 << 16)? else {
            continue;
        };
        let document =
            String::from_utf8(bytes).map_err(|_| "a promise is not UTF-8".to_owned())?;
        promises.push(submit::check_promise(
            &document,
            log,
            source,
            &format!("sha256:{leaf}"),
        )?);
    }
    Ok(promises)
}

/// The registry packages of the project's lock, as their provenance must show them; or the
/// problem of each at its lock line (U7010).
fn expected_packages(project: &Project) -> (Vec<(Expected, &ita::LockPackage)>, Vec<ProjectError>) {
    let mut expected = Vec::new();
    let mut problems = Vec::new();
    for locked in &project.lock.read.packages {
        if registry_source(locked).is_none() {
            continue;
        }
        match vendor::expected(&project.lock.read, project.manifest.ita.as_deref(), locked) {
            Ok(one) => expected.push((one, locked)),
            Err(message) => problems.push(problem(Code::U7010, locked.range.clone(), message)),
        }
    }
    (expected, problems)
}

/// Writes `bytes` to `file` through a fresh file beside it and a rename, so a reader never sees
/// part of it.
fn write_file(file: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("bundle");
    let temporary = file.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut handle = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        handle.write_all(bytes)?;
        handle.sync_all()?;
        std::fs::rename(&temporary, file)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// `una ita bundle <file> [path]` (ADR 0122 rule 33): reads each source of the lock's registry
/// packages at the lock's resolution point, from its first place that verifies (rule 40), and
/// writes into one file, for an air-gapped machine, each package's archive and provenance and
/// every file of the registry the client read to verify them: the root chain, the checkpoints,
/// tiles and map tiles of each name's state and owner's keys. A source no place serves verified
/// is U7010 or U7013, a release the registry does not hold at that point U7010; then nothing is
/// written. O(bytes of the packages and of the files read).
pub(crate) fn bundle_command(
    file: &Path,
    path: &Path,
    itaoca: Option<&Itaoca>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    let project = match project(path) {
        Ok(project) => project,
        Err(ended) => return ended.finish("bundle", out),
    };
    let sources = &project.package.sources;
    let (expected, mut problems) = expected_packages(&project);
    let mirrors = match crate::registry::load_mirrors(itaoca) {
        Ok(mirrors) => mirrors,
        Err(errors) => {
            let diagnostics = located(sources, project.manifest_id, errors);
            return finish("bundle", &[], &[], &diagnostics, sources, out);
        }
    };
    let read = project.manifest.ita.as_deref();
    let mut packed = Vec::new();
    let mut registries: Vec<(String, BTreeMap<String, Vec<u8>>)> = Vec::new();
    let by_source: BTreeMap<&str, Vec<&(Expected, &ita::LockPackage)>> =
        expected.iter().fold(BTreeMap::new(), |mut map, one| {
            map.entry(one.0.source.as_str()).or_default().push(one);
            map
        });
    for (source, packages) in by_source {
        let first_range = packages.first().map_or(0..0, |(_, locked)| locked.range.clone());
        let Some(declared) = read.and_then(|read| crate::registry::declared(read, source)) else {
            problems.push(problem(
                Code::U7010,
                first_range,
                format!("`{source}` is not a source of ita.una's `sources`"),
            ));
            continue;
        };
        match bundle_source(declared, &packages, &mirrors, itaoca) {
            Ok((one_source, files)) => {
                packed.extend(one_source);
                registries.push((source.to_owned(), files));
            }
            Err((code, at, why)) => {
                let range = packages
                    .get(at)
                    .map_or(first_range, |(_, locked)| locked.range.clone());
                problems.push(problem(code, range, why));
            }
        }
    }
    if !problems.is_empty() {
        let diagnostics = located(sources, project.lock_id, problems);
        return finish("bundle", &[], &[], &diagnostics, sources, out);
    }
    let count = packed.len();
    let bytes = match bundle::entries(&packed, &registries).and_then(bundle::write) {
        Ok(bytes) => bytes,
        Err(why) => {
            let diagnostics =
                located(sources, project.lock_id, vec![problem(Code::U7010, 0..0, why)]);
            return finish("bundle", &[], &[], &diagnostics, sources, out);
        }
    };
    if let Err(error) = write_file(file, &bytes) {
        return usage(out, &format!("cannot write `{}`: {error}", file.display()));
    }
    let shown = file.display().to_string();
    let lines = vec![format!("bundled {count} registry packages into {shown}")];
    finish("bundle", &[shown.as_str()], &lines, &[], sources, out)
}

/// What a bundle takes of one source: each package's archive and provenance, and the files of
/// the registry the client read; or the code, the package's index and why not.
type Bundled = (
    Vec<(Expected, bundle::Packed)>,
    BTreeMap<String, Vec<u8>>,
);

/// Reads `declared`'s registry for `packages`, all of one source, at the lock's resolution
/// point, from its first place that verifies, through a [`bundle::Recording`] of that place's
/// transport.
fn bundle_source(
    declared: &ita::Source,
    packages: &[&(Expected, &ita::LockPackage)],
    mirrors: &una_project::mirrors::Mirrors,
    itaoca: Option<&Itaoca>,
) -> Result<Bundled, (Code, usize, String)> {
    let Some((first, _)) = packages.first() else {
        return Ok((Vec::new(), BTreeMap::new()));
    };
    let point = Point::At {
        size: first.checkpoint_size,
        hash: first.checkpoint_hash.clone(),
    };
    let places = crate::registry::places_of(declared, mirrors);
    let mut reasons: Vec<String> = Vec::new();
    for (from, place) in &places {
        let transport = match place {
            Ok(transport) => transport.as_ref(),
            Err(why) => {
                reasons.push(format!("`{from}`: {why}"));
                continue;
            }
        };
        let recording = bundle::Recording::new(transport);
        let mut registry = match crate::registry::open(declared, &recording, &point, itaoca, None)
        {
            Ok(registry) => registry,
            Err(why) => {
                reasons.push(format!("`{from}`: {why}"));
                continue;
            }
        };
        let mut out = Vec::new();
        for (at, (one, _)) in packages.iter().enumerate() {
            let fail = |why: String| (Code::U7010, at, why);
            let version = Version::parse_full(&one.version).map_err(fail)?;
            let release = registry.release(&one.name, &version).map_err(fail)?;
            registry.marks(&one.name).map_err(fail)?;
            if release.index != one.logged || release.statement.digest != one.digest {
                return Err(fail(format!(
                    "the registry's `{} {}` is not the lock's: its log index or digest differs",
                    one.name, one.version
                )));
            }
            let key = release
                .signers
                .iter()
                .find(|key| {
                    una_project::provenance::signed_name(&one.name, &key.public) == one.signed
                })
                .cloned()
                .ok_or_else(|| fail(format!("no signer of `{}` is the key the lock names", one.name)))?;
            let provenance = registry.provenance(&release, &key).map_err(fail)?;
            una_project::provenance::verify(&provenance, one).map_err(fail)?;
            let archive = registry.archive(&one.digest).map_err(fail)?;
            let tree = VerifiedTree::from_archive(&archive).map_err(fail)?;
            if tree.digest() != one.digest {
                return Err(fail(format!(
                    "the archive of `{} {}` is not the lock's",
                    one.name, one.version
                )));
            }
            out.push((
                one.clone(),
                bundle::Packed {
                    archive,
                    provenance: provenance.encode(),
                },
            ));
        }
        drop(registry);
        // The archives travel as packages, not twice as registry files.
        let files: BTreeMap<String, Vec<u8>> = recording
            .seen
            .into_inner()
            .into_iter()
            .filter(|(path, _)| !path.starts_with("blobs/"))
            .collect();
        return Ok((out, files));
    }
    Err((
        Code::U7013,
        0,
        format!(
            "no place of the source `{}` serves its registry at the lock's resolution point: {}",
            declared.name.value,
            reasons.join("; ")
        ),
    ))
}

/// `una ita unbundle <file> [path]` (ADR 0122 rule 33): verifies every registry package of the
/// lock in the bundle, against the lock and the pins of `ita.una`, as every load does, and only
/// when all verify, stores them in Itaoca. No clock freshness is asked. A package the bundle
/// does not hold, and that is not stored already, is U7011; any other failure U7010; then
/// nothing is written.
pub(crate) fn unbundle_command(
    file: &Path,
    path: &Path,
    itaoca: Option<&Itaoca>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    let Some(itaoca) = itaoca else {
        return usage(out, "no Itaoca to fill: set `UNA_ITAOCA` or `HOME`");
    };
    let bytes = match std::fs::metadata(file) {
        Ok(meta) if meta.len() <= bundle::MAX_BYTES => match std::fs::read(file) {
            Ok(bytes) => bytes,
            Err(error) => return usage(out, &format!("cannot read `{}`: {error}", file.display())),
        },
        Ok(_) => return usage(out, &format!("`{}` is larger than a bundle may be", file.display())),
        Err(error) => return usage(out, &format!("cannot read `{}`: {error}", file.display())),
    };
    let project = match project(path) {
        Ok(project) => project,
        Err(ended) => return ended.finish("unbundle", out),
    };
    let sources = &project.package.sources;
    let (expected, mut problems) = expected_packages(&project);
    // A package Itaoca holds already, verified, needs nothing from the bundle.
    let mut wanted: Vec<(Expected, &ita::LockPackage)> = Vec::new();
    for (one, locked) in expected {
        match vendor::stored(itaoca, &one) {
            Ok(Some(_)) => {}
            _ => wanted.push((one, locked)),
        }
    }
    if problems.is_empty() {
        let list: Vec<Expected> = wanted.iter().map(|(one, _)| one.clone()).collect();
        match bundle::unbundle(&bytes, itaoca, &list) {
            Ok(done) => {
                let mut lines: Vec<String> = done
                    .stored
                    .iter()
                    .map(|what| format!("stored {what} in Itaoca"))
                    .collect();
                if !done.unused.is_empty() {
                    lines.push(format!(
                        "{} entries of the bundle were not used and not stored",
                        done.unused.len()
                    ));
                }
                if done.stored.is_empty() {
                    lines.push("Itaoca holds every locked registry package already".to_owned());
                }
                return finish("unbundle", &[], &lines, &[], sources, out);
            }
            Err((index, failure)) => {
                let (code, why) = match failure {
                    vendor::Problem::Absent(why) => (Code::U7011, why),
                    vendor::Problem::Refused(why) => (Code::U7010, why),
                };
                let range = index
                    .and_then(|index| wanted.get(index))
                    .map_or(0..0, |(_, locked)| locked.range.clone());
                problems.push(problem(code, range, why));
            }
        }
    }
    let diagnostics = located(sources, project.lock_id, problems);
    finish("unbundle", &[], &[], &diagnostics, sources, out)
}
