//! `una ita publish`, `yank`, `audit`, `watch`, `bundle` and `unbundle` (ADR 0122 rules 33, 36,
//! 36.2 and 40 to 43; `spec/30 §14`).
//!
//! A submission goes out only through a transport, and this release has only the directory
//! transport: `--upload` takes a `file:///` URL, and an `https://` one is refused as having no
//! transport, plain `http://` as never used. What a command needs to know of a registry (a name's
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
use una_project::registry::keyfile::SigningKey;
use una_project::registry::transport::{Directory, Failure as TransportFailure, Transport};
use una_project::registry::view::{Failure, Registry};
use una_project::registry::{audit, bundle, submit, watch};
use una_project::version::Version;
use una_project::{tree, vendor};
use una_syntax::{FileId, SourceDb, ita};

use crate::ExitStatus;
use crate::ita::{Output, finish, located};
use crate::verification::Solving;

/// Why a command that needs the verified registry client stops in this release.
const NO_CLIENT: &str = "this release has no registry client to verify a registry's roots, checkpoints, name map and chains";

/// The most bytes of a key file.
const MAX_KEY_FILE: u64 = 4096;

/// The verified registry client (ADR 0122 rules 35 to 39) that publishing, yanks, audit and
/// watch read through. It joins with the verification of roots, checkpoints, the name map and
/// proofs; until then there is none.
pub(crate) fn client(_itaoca: Option<&Itaoca>) -> Option<Box<dyn Registry>> {
    None
}

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

/// Reads a publisher's key file (ADR 0122 rule 38): a regular file, not a link, of at most
/// [`MAX_KEY_FILE`] bytes, and on Unix readable by its owner only.
fn read_key(path: &Path) -> Result<SigningKey, String> {
    let shown = path.display();
    let meta = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot read the key file `{shown}`: {error}"))?;
    if !meta.is_file() {
        return Err(format!("the key file `{shown}` is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(format!(
                "the key file `{shown}` can be read by others; make it readable by its owner only (mode 0600)"
            ));
        }
    }
    if meta.len() > MAX_KEY_FILE {
        return Err(format!("the key file `{shown}` is too long"));
    }
    let bytes = tree::read_unlinked(path)?;
    let text = String::from_utf8(bytes).map_err(|_| format!("the key file `{shown}` is not UTF-8"))?;
    SigningKey::parse(&text).map_err(|why| format!("the key file `{shown}`: {why}"))
}

/// What `publish` and `yank` sign with and send to.
pub(crate) struct Signing<'a> {
    /// `--key <file>`, each a key of the owner, in order.
    pub(crate) keys: &'a [PathBuf],
    /// `--upload <url>`: a `file:///` URL of the registry's directory.
    pub(crate) upload: Option<&'a str>,
}

impl Signing<'_> {
    /// The keys read and the transport, or the usage error.
    fn open(&self) -> Result<(Vec<SigningKey>, Directory), String> {
        let upload = self.upload.ok_or(
            "this release sends a submission only to a directory: give `--upload file:///<dir>`",
        )?;
        let transport = Directory::of(upload)?;
        if self.keys.is_empty() {
            return Err("give the owner's signing keys with `--key <file>`".to_owned());
        }
        let keys = self
            .keys
            .iter()
            .map(|path| read_key(path))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((keys, transport))
    }
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

/// The name state of the package's name from the client, or the problem at `range`: U7010 for
/// a failure to verify, U7013 when there is no state to publish against.
fn name_state(
    registry: Option<&dyn Registry>,
    source: &str,
    name: &str,
    range: std::ops::Range<usize>,
) -> Result<una_project::registry::view::NameState, ProjectError> {
    let Some(registry) = registry else {
        return Err(problem(
            Code::U7013,
            range,
            format!(
                "the chain of `{name}` at `{source}` cannot be read and verified, since {NO_CLIENT}; nothing is sent"
            ),
        ));
    };
    registry.name(source, name, 0).map_err(|failure| match failure {
        Failure::Refused(why) => problem(
            Code::U7010,
            range,
            format!("the state of `{name}` at `{source}` does not verify: {why}"),
        ),
        Failure::Unavailable(why) => problem(
            Code::U7013,
            range,
            format!("the state of `{name}` at `{source}` cannot be had: {why}; nothing is sent"),
        ),
    })
}

/// `una ita publish [path] --key <file>... --upload <url>` (ADR 0122 rule 43): refuses what
/// rule 43 names, checks the package at tier 1 and refuses any diagnostic, builds its archive
/// (rule 27), writes the release statement against the verified state of its name (rule 26
/// against the highest earlier release of its line), signs it with the keys given, and sends the
/// archive and the statement through the directory transport. The submission is recorded in
/// Itaoca, so `una ita watch` can check the registry's promise for it. Costs: the check, and
/// O(B) for the B bytes of the package.
pub(crate) fn publish_command(
    path: &Path,
    signing: Signing<'_>,
    solving: &Solving,
    registry: Option<&dyn Registry>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    if !path.join("ita.una").is_file() {
        return usage(out, &not_a_package(path));
    }
    let (keys, transport) = match signing.open() {
        Ok(opened) => opened,
        Err(message) => return usage(out, &message),
    };
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
    let state = match name_state(registry, &source, &name, head.clone()) {
        Ok(state) => state,
        Err(refusal) => {
            diagnostics.extend(located(sources, manifest_id, vec![refusal]));
            return finish("publish", &[], &[], &diagnostics, sources, out);
        }
    };
    let (project, _) = crate::package::project(&package);
    let trust: std::collections::BTreeSet<String> =
        project.trusted.iter().map(|t| t.value.clone()).collect();
    let signers: Vec<&SigningKey> = keys.iter().collect();
    let submission = match submit::release(
        read,
        files,
        &authority(&analysis),
        &trust,
        &state,
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
    send_and_record(
        "publish",
        &submission,
        &transport,
        &source,
        solving.itaoca.as_ref(),
        format!(
            "published {name} {} to `{}`",
            read.version.as_ref().map(|v| v.value.as_str()).unwrap_or_default(),
            transport.name()
        ),
        (sources, manifest_id, head),
        out,
    )
}

/// Sends a submission, records it in Itaoca for `una ita watch`, and prints what was done.
#[allow(clippy::too_many_arguments)]
fn send_and_record(
    command: &str,
    submission: &submit::Submission,
    transport: &Directory,
    source: &str,
    itaoca: Option<&Itaoca>,
    done: String,
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
    let mut lines = vec![
        done,
        format!(
            "the log will hold the leaf {}; `una ita watch` checks the registry's promise to log it",
            submission.leaf
        ),
    ];
    if let Some(itaoca) = itaoca
        && let Err(why) = record_submission(itaoca, source, submission, transport.name())
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

/// `una ita yank <version> [path] --key <file>... --upload <url>` (ADR 0122 rule 41): writes a
/// yank of a release of the package at `path`, against the verified state of its name, signs it
/// with the keys given, which must be the owner's threshold for it to stand, and sends it
/// through the directory transport.
pub(crate) fn yank_command(
    version: &str,
    path: &Path,
    signing: Signing<'_>,
    itaoca: Option<&Itaoca>,
    registry: Option<&dyn Registry>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    let manifest_path = path.join("ita.una");
    if !manifest_path.is_file() {
        return usage(out, &not_a_package(path));
    }
    let Ok(version) = Version::parse_full(version) else {
        return usage(out, &format!("`{version}` is not a full version, such as `0.3.1`"));
    };
    let (keys, transport) = match signing.open() {
        Ok(opened) => opened,
        Err(message) => return usage(out, &message),
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
    let state = match name_state(registry, &source, &name, head.clone()) {
        Ok(state) => state,
        Err(refusal) => {
            diagnostics.extend(located(&sources, id, vec![refusal]));
            return finish("yank", &[], &[], &diagnostics, &sources, out);
        }
    };
    let signers: Vec<&SigningKey> = keys.iter().collect();
    let submission = match submit::yank(&state, &version, &signers) {
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
    send_and_record(
        "yank",
        &submission,
        &transport,
        &source,
        itaoca,
        format!("yanked {name} {version} at `{}`", transport.name()),
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
        }) => Some(source),
        _ => None,
    }
}

/// The diagnostics of `una ita audit` on `project` against `registry`, and the lines it prints.
fn audit_project(project: &Project, registry: Option<&dyn Registry>) -> (Vec<Diagnostic>, Vec<String>) {
    let sources = &project.package.sources;
    let mut problems: Vec<ProjectError> = Vec::new();
    let mut at_needs: Vec<ProjectError> = Vec::new();
    let mut packages: Vec<audit::Locked> = Vec::new();
    let mut lines_of: Vec<&ita::LockPackage> = Vec::new();
    for locked in &project.lock.read.packages {
        let Some(source) = registry_source(locked) else {
            continue;
        };
        let name = locked.name.value.to_string();
        let Ok(version) = Version::parse_full(&locked.version.value) else {
            continue;
        };
        if registry.is_none() {
            problems.push(problem(
                Code::U7010,
                locked.range.clone(),
                format!(
                    "the state of `{name}` at `{source}` cannot be verified, since {NO_CLIENT}; the audit reports nothing as clean that it did not verify"
                ),
            ));
            continue;
        }
        let resolution = project
            .lock
            .read
            .registries
            .iter()
            .find(|r| r.source.value == source)
            .map_or(0, |r| r.checkpoint_size.value);
        packages.push(audit::Locked {
            source: source.to_owned(),
            name,
            version,
            signed: locked.signed.as_ref().map(|s| s.value.clone()),
            logged: locked.logged.as_ref().map(|l| l.value),
            resolution,
        });
        lines_of.push(locked);
    }
    let mut lines = Vec::new();
    if let Some(registry) = registry {
        match audit::audit(&packages, registry) {
            Err((index, failure)) => {
                let why = match failure {
                    Failure::Refused(why) => why,
                    Failure::Unavailable(why) => format!("no fresh, verified state: {why}"),
                };
                if let (Some(locked), Some(package)) = (lines_of.get(index), packages.get(index)) {
                    problems.push(problem(
                        Code::U7010,
                        locked.range.clone(),
                        format!(
                            "the state of `{}` at `{}` cannot be verified: {why}",
                            package.name, package.source
                        ),
                    ));
                }
            }
            Ok(result) => {
                let mut by_package: BTreeMap<usize, Vec<String>> = BTreeMap::new();
                for finding in &result.findings {
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
                        Some(need) => at_needs.push(problem(Code::U7014, need.range.clone(), message)),
                        None => problems.push(problem(Code::U7014, locked.range.clone(), message)),
                    }
                }
                if result.findings.is_empty() {
                    lines.push(format!(
                        "audited {} registry packages: none is yanked, advised, or of an owner whose keys changed",
                        packages.len()
                    ));
                }
            }
        }
    } else if problems.is_empty() {
        lines.push("no registry package is locked".to_owned());
    }
    let mut diagnostics = located(sources, project.manifest_id, at_needs);
    diagnostics.extend(located(sources, project.lock_id, problems));
    (diagnostics, lines)
}

/// `una ita audit [path]` (ADR 0122 rule 41, U7014): each registry package of the lock against
/// the verified state of its name and its owner at a fresh checkpoint. Exit code 1 on any
/// finding, and on any state that cannot be verified.
pub(crate) fn audit_command(
    path: &Path,
    registry: Option<&dyn Registry>,
    out: Output<'_>,
) -> io::Result<ExitStatus> {
    let project = match project(path) {
        Ok(project) => project,
        Err(ended) => return ended.finish("audit", out),
    };
    let (diagnostics, lines) = audit_project(&project, registry);
    finish("audit", &[], &lines, &diagnostics, &project.package.sources, out)
}

/// The codes `una ita audit` reports for the package at `path` against `registry`.
pub(crate) fn audit_codes(path: &Path, registry: &dyn Registry) -> Vec<Code> {
    match project(path) {
        Ok(project) => audit_project(&project, Some(registry))
            .0
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect(),
        Err(ended) => ended.codes(),
    }
}

/// `una ita watch <owner> [<name>...] [--from <source>]` (ADR 0122 rules 36 and 36.2): prints
/// every entry of the owner's chain and of its packages' chains since the position stored in
/// Itaoca, then stores the new position; and checks the registry's promise for each submission
/// recorded, reporting a broken one. Exit code 1 when a promise is broken or a chain does not
/// verify.
pub(crate) fn watch_command(
    owner: &str,
    names: &[String],
    source: &str,
    itaoca: Option<&Itaoca>,
    registry: Option<&dyn Registry>,
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
    let Some(registry) = registry else {
        writeln!(out.stderr, "error: the chains of `{owner}` at `{source}` cannot be followed, since {NO_CLIENT}")?;
        return Ok(ExitStatus::Errors);
    };
    let position_name = format!("{owner}.position");
    let stored = match tree::locate(&itaoca.root, &["registry", source, "watch", &position_name]) {
        Ok(Some(file)) => match tree::read_file(&file, 1 << 20)
            .and_then(|bytes| String::from_utf8(bytes).map_err(|_| "not UTF-8".to_owned()))
            .and_then(|text| watch::Position::parse(&text))
        {
            Ok(position) => position,
            Err(why) => return usage(out, &format!("the stored position of `{owner}` does not read: {why}")),
        },
        Ok(None) => watch::Position::start(source, owner),
        Err(why) => return usage(out, &why),
    };
    let promises = match recorded_promises(itaoca, source, registry) {
        Ok(promises) => promises,
        Err(why) => {
            writeln!(out.stderr, "error: {why}")?;
            return Ok(ExitStatus::Errors);
        }
    };
    let report = match watch::watch(registry, &stored, names, &promises) {
        Ok(report) => report,
        Err(Failure::Refused(why) | Failure::Unavailable(why)) => {
            writeln!(out.stderr, "error: the chains of `{owner}` at `{source}` cannot be followed: {why}")?;
            return Ok(ExitStatus::Errors);
        }
    };
    let mut lines: Vec<String> = report
        .entries
        .iter()
        .map(|(name, entry)| format!("{name}: {} at log entry {} ({})", entry.format, entry.index, entry.leaf))
        .collect();
    for promise in &report.kept {
        lines.push(format!("kept: the promise to log {} by {}", promise.leaf, promise.by));
    }
    for promise in &report.broken {
        lines.push(format!(
            "broken: the registry promised to log {} by {}, and its checkpoint is later without it; publish the promise as evidence",
            promise.leaf, promise.by
        ));
    }
    if lines.is_empty() {
        lines.push(format!("nothing new for `{owner}` at `{source}`"));
    }
    let mut written: Vec<String> = Vec::new();
    let stored_position = tree::make_dirs(&itaoca.root, &["registry", source, "watch"])
        .and_then(|parent| tree::store_file(&parent, &position_name, report.position.encode().as_bytes(), false));
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
/// and verified with the log's key: a submission whose promise has not appeared yet is skipped.
fn recorded_promises(
    itaoca: &Itaoca,
    source: &str,
    registry: &dyn Registry,
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
    if leaves.is_empty() {
        return Ok(Vec::new());
    }
    let log = registry.log_key(source).map_err(|failure| match failure {
        Failure::Refused(why) | Failure::Unavailable(why) => {
            format!("the log's key of `{source}` cannot be had: {why}")
        }
    })?;
    let mut promises = Vec::new();
    for leaf in leaves {
        let Some(file) = tree::locate(&dir, &[leaf.as_str()])? else {
            continue;
        };
        let url = String::from_utf8(tree::read_file(&file, 4096)?)
            .map_err(|_| "a recorded submission is not UTF-8".to_owned())?;
        let transport = Directory::of(url.trim_end())?;
        let document = match transport.get(&format!("submit/{leaf}/promise"), 1 << 16) {
            Ok(bytes) => String::from_utf8(bytes).map_err(|_| "a promise is not UTF-8".to_owned())?,
            Err(TransportFailure::Absent) => continue,
            Err(TransportFailure::Unavailable(why)) => return Err(why),
        };
        let promise = submit::check_promise(&document, &log, source, &format!("sha256:{leaf}"))?;
        promises.push(promise);
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

/// `una ita bundle <file> [path]` (ADR 0122 rule 33): writes each registry package of the lock,
/// loaded verified from the vendor directory or Itaoca, as its archive with its provenance, into
/// one file for an air-gapped machine. A package in neither place is U7011, one that does not
/// verify U7010; then nothing is written. O(bytes of the packages).
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
    let mut packed = Vec::new();
    for (one, locked) in expected {
        match vendor::load(path, itaoca, &one) {
            Ok(loaded) => {
                let mut files = loaded.tree.into_files();
                match content::archive(&mut files) {
                    Ok(archive) => packed.push((
                        one,
                        bundle::Packed {
                            archive,
                            provenance: loaded.provenance,
                        },
                    )),
                    Err(why) => problems.push(problem(Code::U7010, locked.range.clone(), why)),
                }
            }
            Err(vendor::Problem::Absent(why)) => {
                problems.push(problem(Code::U7011, locked.range.clone(), why));
            }
            Err(vendor::Problem::Refused(why)) => {
                problems.push(problem(Code::U7010, locked.range.clone(), why));
            }
        }
    }
    if !problems.is_empty() {
        let diagnostics = located(sources, project.lock_id, problems);
        return finish("bundle", &[], &[], &diagnostics, sources, out);
    }
    let count = packed.len();
    let bytes = match bundle::entries(&packed, Vec::new()).and_then(bundle::write) {
        Ok(bytes) => bytes,
        Err(why) => {
            let diagnostics = located(sources, project.lock_id, vec![problem(Code::U7010, 0..0, why)]);
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
