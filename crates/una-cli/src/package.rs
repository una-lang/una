//! Loading a package into a source database, and the diagnostics of its project files
//! (ADR 0021, ADR 0022).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use una_diag::{Code, Diagnostic};
use una_project::ita::{check_consent, check_lock_consistency, parse_ita_lock, parse_ita_manifest};
use una_project::itaoca::Itaoca;
use una_project::vendor::Problem;
use una_project::{
    Edition, Format, Manifest, ProjectError, Role, discover, parse_lock, parse_manifest,
};
use una_syntax::ita;
use una_syntax::{FileId, SourceDb, SourceFile, Span};

pub use una_project::LoadError;

/// A loaded package: its files in a source database, by role.
#[derive(Debug)]
pub struct Package {
    /// Every file of the package: `una.toml`, `verification.lock`, then the modules.
    pub sources: SourceDb,
    /// Whether the package is a single file checked on its own (ADR 0021 rule 1).
    pub single: bool,
    /// Which manifest the package has (ADR 0122 rule 2).
    pub format: Format,
    /// Whether the directory holds both `ita.una` and `una.toml` (U7008).
    pub both_manifests: bool,
    /// `una.toml` or `ita.una`, for a package directory.
    pub manifest: Option<FileId>,
    /// `verification.lock` or `ita.lock`, when the package has one.
    pub lock: Option<FileId>,
    /// The lock's bytes exactly as they were read, which `una ita add` and `remove` put back
    /// when `ita.una` cannot be written after the lock.
    pub lock_bytes: Option<Vec<u8>>,
    /// The modules, in path order.
    pub modules: Vec<FileId>,
    /// Where each file is on disk, for `una fmt`.
    pub disk_paths: BTreeMap<FileId, PathBuf>,
    /// For a single file, the edition of the nearest enclosing package manifest, when some
    /// directory above the file has an `ita.una` or `una.toml` (#1175). Only the edition is
    /// taken: the file stays its own package for imports and module paths (ADR 0021 rule 1),
    /// so that `una fmt` and `una check` of a named file read it as the package reads it.
    pub enclosing_edition: Option<Edition>,
    /// The path dependencies of `ita.una`, loaded or not (ADR 0122 rule 7). Their modules are
    /// checked with the root's only once [`project`] mounts them.
    pub dependencies: Vec<Dependency>,
    /// The registry packages of `ita.lock`, each loaded and verified, or why not (ADR 0122
    /// rule 17), which [`project`] reports at its line of the lock.
    pub registry: Vec<RegistryPackage>,
}

/// A registry package of the lock, by its index among the lock's packages, and the problem of
/// its verified load, if any: U7011 when it is in neither the vendor directory nor Itaoca,
/// U7010 when it does not verify.
#[derive(Debug)]
pub struct RegistryPackage {
    /// Its index among the lock's packages.
    pub index: usize,
    /// The problem, with its code, or `None` when it loads verified.
    pub problem: Option<(Code, String)>,
}

/// A dependency of the root: a path dependency, or a registry release of the lock that loaded
/// verified; the need that names it, and its files, or why they could not be read.
#[derive(Debug)]
pub struct Dependency {
    /// The need's index in the root's `needs`.
    pub need: usize,
    /// The directory as `ita.una` writes it, without a trailing `/`: the prefix of its files'
    /// names in the source database.
    pub dir: String,
    /// Its `ita.una`.
    pub manifest: Option<FileId>,
    /// Its modules, in path order.
    pub modules: Vec<FileId>,
    /// Why it could not be loaded.
    pub error: Option<String>,
    /// Each of its files, by its path in the package, with its bytes exactly as they were read
    /// once and loaded: its content digest is computed from these (ADR 0122 rule 28), so the
    /// code checked and the content consented to are the same bytes, never a file read again
    /// by its path.
    pub files: Vec<una_project::content::File>,
    /// For a registry release, where it comes from; `None` for a path dependency.
    pub registry: Option<RegistryOrigin>,
}

impl Dependency {
    /// The edition its `ita.una` names, under which its modules are lexed, parsed and formatted
    /// (ADR 0122 rules 48 and 49); `None` when it has no manifest.
    pub fn edition(&self, sources: &SourceDb) -> Option<Edition> {
        let manifest = self.manifest?;
        Some(parse_ita_manifest(sources.file(manifest)).0.edition)
    }
}

/// Where a mounted registry release comes from (ADR 0122 rules 16 and 58).
#[derive(Clone, Debug)]
pub struct RegistryOrigin {
    /// The source.
    pub source: String,
    /// Its index among the lock's packages.
    pub lock: usize,
}

/// Loads the package at `path` (ADR 0021 rule 1). Its registry packages are looked for in its
/// vendor directory only: a command that knows Itaoca loads with [`load_in`].
pub fn load(path: &Path) -> Result<Package, LoadError> {
    load_in(path, &BTreeMap::new(), None)
}

/// Loads the package at `path`, with the text of `overlay` in place of the files it names:
/// the unsaved buffers of an editor (ADR 0063 rule 4).
pub fn load_with(path: &Path, overlay: &BTreeMap<PathBuf, String>) -> Result<Package, LoadError> {
    load_in(path, overlay, None)
}

/// [`load_with`], with the registry packages of the lock looked for in the vendor directory,
/// then in `itaoca`, and each verified as it is read (ADR 0122 rules 17, 30 and 32). No
/// connection is opened. O(bytes of the packages read).
pub fn load_in(
    path: &Path,
    overlay: &BTreeMap<PathBuf, String>,
    itaoca: Option<&Itaoca>,
) -> Result<Package, LoadError> {
    let mut loaded = discover(path)?;
    for file in &mut loaded.files {
        if let Some(text) = overlay.get(&file.path) {
            file.bytes = text.as_bytes().to_vec();
            file.len = u64::try_from(file.bytes.len()).unwrap_or(u64::MAX);
        }
    }
    let mut package = Package {
        sources: SourceDb::new(),
        single: loaded.single,
        format: loaded.format,
        both_manifests: loaded.both_manifests,
        manifest: None,
        lock: None,
        lock_bytes: None,
        modules: Vec::new(),
        disk_paths: BTreeMap::new(),
        enclosing_edition: if loaded.single {
            enclosing_edition(path)
        } else {
            None
        },
        dependencies: Vec::new(),
        registry: Vec::new(),
    };
    for file in loaded.files {
        // A project file over its bound was left unread and keeps only its length, which its
        // reader refuses (spec/30 §1 rule 4).
        let id = package.sources.insert(file.decode());
        package.disk_paths.insert(id, file.path);
        match file.role {
            Role::Manifest => package.manifest = Some(id),
            Role::Lock => {
                package.lock = Some(id);
                package.lock_bytes = Some(file.bytes);
            }
            Role::Module => package.modules.push(id),
        }
    }
    if package.format == Format::Ita
        && let Some(id) = package.manifest
    {
        // The files as decoded once, by the source database (#2252).
        let manifest = package.sources.file(id).clone();
        load_dependencies(path, &manifest, overlay, &mut package);
        if let Some(lock) = package.lock {
            let lock = package.sources.file(lock).clone();
            load_registry(path, &manifest, &lock, itaoca, &mut package);
        }
    }
    Ok(package)
}

/// Loads and verifies each registry package of the lock (ADR 0122 rule 17), when the lock reads
/// without an error: a lock that does not read is U7009 already, and names nothing to trust.
/// A release that loads verified and that a need of the root names is added to the package as
/// a dependency, its files the verified bytes read once, under `vendor/ita/<source>/<release>/`
/// or `itaoca/<source>/<release>/` in the source database, which [`project`] mounts.
fn load_registry(
    root: &Path,
    manifest: &SourceFile,
    lock: &SourceFile,
    itaoca: Option<&Itaoca>,
    package: &mut Package,
) {
    let (read, errors) = parse_ita_lock(lock);
    if !errors.is_empty() {
        return;
    }
    let (manifest, _) = una_syntax::ita::parse_manifest(manifest);
    let mut registry = Vec::new();
    for (index, locked) in read.read.packages.iter().enumerate() {
        let Some(ita::At {
            value: ita::LockOrigin::Source(source),
            ..
        }) = &locked.from
        else {
            continue;
        };
        // Until the root of `itaty` is built into `una` (ADR 0122 rule 35), a package of it can
        // be neither verified nor needed: its need is refused already (U7008, `spec/30 §3`
        // rule 4), so it never loads. Every other source is checked.
        if source == una_project::ita::ITATY {
            continue;
        }
        let problem = match una_project::vendor::load_locked(
            root,
            itaoca,
            &read.read,
            Some(&manifest),
            locked,
        ) {
            Ok(loaded) => {
                mount_registry(package, &manifest, locked, source, index, &loaded);
                None
            }
            Err(Problem::Absent(message)) => Some((Code::U7011, message)),
            Err(Problem::Refused(message)) => Some((Code::U7010, message)),
            Err(Problem::Unavailable(message)) => Some((Code::U7013, message)),
        };
        registry.push(RegistryPackage { index, problem });
    }
    package.registry = registry;
}

/// Adds the verified release `loaded`, the lock's package `index` of `source`, as a dependency
/// of the root, when a need of the root names it: its name, its source, and its major line.
fn mount_registry(
    package: &mut Package,
    manifest: &ita::Manifest,
    locked: &ita::LockPackage,
    source: &str,
    index: usize,
    loaded: &una_project::vendor::Loaded,
) {
    let Ok(version) = una_project::version::Version::parse_full(&locked.version.value) else {
        return;
    };
    let need = manifest.needs.iter().position(|need| {
        need.name.value == locked.name.value
            && matches!(
                need.origin.as_ref().map(|o| &o.value),
                Some(ita::Origin::Source(s)) if s == source
            )
            && need
                .version
                .as_ref()
                .and_then(|v| una_project::version::Version::parse(&v.value).ok())
                .is_some_and(|written| written.version.line() == version.line())
    });
    let Some(need) = need else {
        return;
    };
    let Ok(release) =
        una_project::vendor::release_dir(&locked.name.value.to_string(), &locked.version.value)
    else {
        return;
    };
    let place = match loaded.place {
        una_project::vendor::Place::Vendor => "vendor/ita",
        una_project::vendor::Place::Itaoca => "itaoca",
    };
    let dir = format!("{place}/{source}/{release}");
    let mut dependency = Dependency {
        need,
        dir: dir.clone(),
        manifest: None,
        modules: Vec::new(),
        error: None,
        // The verified release's files, as read once: its content digest is the one verified.
        files: loaded.tree.files().to_vec(),
        registry: Some(RegistryOrigin {
            source: source.to_owned(),
            lock: index,
        }),
    };
    for file in loaded.tree.files() {
        if file.path == "ita.una" {
            let id = package.sources.add(format!("{dir}/ita.una"), &file.bytes);
            dependency.manifest = Some(id);
        } else if file.path.starts_with("src/") && file.path.ends_with(".una") {
            let id = package
                .sources
                .add(format!("{dir}/{}", file.path), &file.bytes);
            dependency.modules.push(id);
        }
    }
    package.dependencies.push(dependency);
}

/// A package of edition 0.4 from its files in memory, by their paths in the package: its
/// `ita.una` and its modules under `src/`, in path order, with no lock and no dependency. It
/// checks the code of a verified archive (ADR 0122 rule 37) from the bytes that were verified,
/// never read from disk again.
pub fn load_files(files: &[(String, String)]) -> Result<Package, LoadError> {
    let mut package = Package {
        sources: SourceDb::new(),
        single: false,
        format: Format::Ita,
        both_manifests: false,
        manifest: None,
        lock: None,
        lock_bytes: None,
        modules: Vec::new(),
        disk_paths: BTreeMap::new(),
        enclosing_edition: None,
        dependencies: Vec::new(),
        registry: Vec::new(),
    };
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort();
    for (path, text) in sorted {
        if path == "ita.una" {
            package.manifest = Some(package.sources.add(path.clone(), text.as_bytes()));
        } else if path.starts_with("src/") && path.ends_with(".una") {
            let id = package.sources.add(path.clone(), text.as_bytes());
            package.modules.push(id);
        }
    }
    if package.manifest.is_none() {
        return Err(LoadError::NotAPackage(PathBuf::from("ita.una")));
    }
    Ok(package)
}

/// The edition of the nearest manifest above the file `path`: the first directory among its
/// ancestors holding `ita.una` (read as such) or else `una.toml`. `None` for a loose file, with
/// no manifest in any ancestor, which keeps the ADR 0021 default, edition 0.1 (#1175). The
/// manifest is read only for its edition. O(depth of the path).
fn enclosing_edition(path: &Path) -> Option<Edition> {
    let absolute = std::fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .ok()?;
    for dir in absolute.ancestors().skip(1) {
        let ita = dir.join("ita.una");
        if ita.is_file() {
            // Read within its bound (spec/30 §1 rule 4).
            let file =
                una_project::read_project_file(&ita, "ita.una", ita::MAX_MANIFEST_BYTES).ok()?;
            return Some(parse_ita_manifest(&file).0.edition);
        }
        let toml = dir.join("una.toml");
        if toml.is_file() {
            let text = std::fs::read(&toml).ok()?;
            return Some(parse_manifest(&String::from_utf8_lossy(&text)).0.edition);
        }
    }
    None
}

/// Loads the path dependencies that the root's `ita.una` names (ADR 0122 rule 7). A directory
/// that is not relative, that is the root, lies under the root's `src/`, or is named by two
/// needs is not read; nor is one that does not load. Each keeps the reason, which [`project`]
/// reports at its need. O(files of the dependencies).
fn load_dependencies(
    root: &Path,
    manifest: &SourceFile,
    overlay: &BTreeMap<PathBuf, String>,
    package: &mut Package,
) {
    let (read, _) = una_syntax::ita::parse_manifest(manifest);
    let canonical_root = std::fs::canonicalize(root).ok();
    let mut seen: Vec<PathBuf> = Vec::new();
    for (index, need) in read.needs.iter().enumerate() {
        let Some(una_syntax::ita::At {
            value: una_syntax::ita::Origin::Path(dir),
            ..
        }) = &need.origin
        else {
            continue;
        };
        let written = dir.trim_end_matches('/').to_owned();
        let mut dependency = Dependency {
            need: index,
            dir: written.clone(),
            manifest: None,
            modules: Vec::new(),
            error: None,
            files: Vec::new(),
            registry: None,
        };
        let refusal = (|| {
            if !una_project::ita::is_relative_dir(&written) || written == "." {
                return Some(
                    "its directory is not a relative path to another directory".to_owned(),
                );
            }
            let place = root.join(&written);
            let canonical = std::fs::canonicalize(&place).ok()?;
            if let Some(root) = &canonical_root
                && (canonical == *root || canonical.starts_with(root.join("src")))
            {
                return Some(
                    "its directory is the package itself, or inside its `src/`".to_owned(),
                );
            }
            if seen.contains(&canonical) {
                return Some("another need names the same directory".to_owned());
            }
            seen.push(canonical);
            None
        })();
        if let Some(reason) = refusal {
            dependency.error = Some(reason);
            package.dependencies.push(dependency);
            continue;
        }
        match una_project::discover_dependency(&root.join(&written), &written) {
            Ok(loaded) => {
                for mut file in loaded.files {
                    if let Some(text) = overlay.get(&file.path) {
                        file.bytes = text.as_bytes().to_vec();
                        file.len = u64::try_from(file.bytes.len()).unwrap_or(u64::MAX);
                    }
                    let relative = file
                        .name
                        .strip_prefix(&format!("{written}/"))
                        .unwrap_or(&file.name)
                        .to_owned();
                    let id = package.sources.insert(file.decode());
                    package.disk_paths.insert(id, file.path);
                    match file.role {
                        Role::Manifest => dependency.manifest = Some(id),
                        Role::Module => dependency.modules.push(id),
                        Role::Lock => {}
                    }
                    if file.role != Role::Lock {
                        dependency.files.push(una_project::content::File {
                            path: relative,
                            bytes: file.bytes,
                        });
                    }
                }
            }
            Err(error) => dependency.error = Some(error.to_string()),
        }
        package.dependencies.push(dependency);
    }
}

/// The settings of the package and the diagnostics of its project files. A single file has
/// the default settings and lock (ADR 0021); an invalid `una.toml` still yields its settings
/// that parsed.
pub fn project(package: &Package) -> (Manifest, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let mut located = |file: FileId, errors: Vec<ProjectError>| {
        for error in errors {
            let span = Span::new(file, error.range.start, error.range.end);
            diagnostics.push(Diagnostic::error(
                error.code,
                error.message,
                package.sources.location(span),
            ));
        }
    };
    if package.format == Format::Ita {
        let mut manifest = match package.manifest {
            Some(id) => {
                let (manifest, errors) = parse_ita_manifest(package.sources.file(id));
                located(id, errors);
                if package.both_manifests {
                    located(
                        id,
                        vec![ProjectError {
                            code: Code::U7008,
                            range: 0..0,
                            message: "the package has both ita.una and una.toml; remove una.toml, whose settings ita.una replaces".to_owned(),
                        }],
                    );
                }
                manifest
            }
            None => Manifest::default(),
        };
        let Some(manifest_id) = package.manifest else {
            return (manifest, diagnostics);
        };
        let read = manifest.ita.clone();
        let needs: &[ita::Need] = read.as_deref().map_or(&[], |read| read.needs.as_slice());
        let root_name = read
            .as_deref()
            .and_then(|read| read.name.as_ref())
            .map(|name| name.value.to_string())
            .unwrap_or_default();
        // The lock as written, for the consents to trust; its errors are reported below.
        let lock_read = package
            .lock
            .map(|id| parse_ita_lock(package.sources.file(id)).0);
        // The path dependencies: checked, then mounted with their package extents.
        let mut versions: BTreeMap<String, String> = BTreeMap::new();
        for (index, need) in needs.iter().enumerate() {
            let refuse = |message: String| ProjectError {
                code: Code::U7008,
                range: need.range.clone(),
                message,
            };
            // A path dependency, or a release of a source of `sources` that the lock pins and
            // that loaded verified (`spec/30 §14`). A need of `itaty`, whose root is not built
            // into this release, or of a workspace member is refused, so no import reaches a
            // package that is not checked. A registry need whose release did not load is
            // reported at its line of the lock (U7009, U7010, U7011), and is not mounted.
            let registry_need = match need.origin.as_ref().map(|o| &o.value) {
                Some(ita::Origin::Path(_)) => false,
                Some(ita::Origin::Source(source))
                    if source != una_project::ita::ITATY
                        && read
                            .as_deref()
                            .is_some_and(|r| r.sources.iter().any(|s| s.name.value == *source)) =>
                {
                    true
                }
                _ => {
                    located(
                        manifest_id,
                        vec![refuse(format!(
                            "this release loads path dependencies and the packages of the sources of `sources`, so `{}` cannot be needed",
                            need.name.value
                        ))],
                    );
                    continue;
                }
            };
            let Some(dependency) = package
                .dependencies
                .iter()
                .find(|d| d.need == index && d.registry.is_some() == registry_need)
            else {
                continue;
            };
            if let Some(reason) = &dependency.error {
                located(
                    manifest_id,
                    vec![refuse(format!(
                        "the path dependency `{}` cannot be loaded: {reason}",
                        dependency.dir
                    ))],
                );
                continue;
            }
            let Some(dependency_manifest) = dependency.manifest else {
                continue;
            };
            let (settings, errors) = parse_ita_manifest(package.sources.file(dependency_manifest));
            let invalid = !errors.is_empty();
            located(dependency_manifest, errors);
            let mut problems =
                dependency_problems(need, &settings, &manifest, dependency.registry.is_none());
            if dependency.registry.is_some() && !settings.trusted.is_empty() {
                problems.push(refuse(format!(
                    "`{}` has trusted modules, which this release does not load for a registry package yet",
                    need.name.value
                )));
            }
            if invalid {
                problems.push(refuse(format!(
                    "the path dependency `{}` has an invalid ita.una",
                    dependency.dir
                )));
            }
            if !problems.is_empty() {
                located(manifest_id, problems);
                continue;
            }
            if let Some(version) = &settings.version
                && dependency.registry.is_none()
            {
                versions.insert(dependency.dir.clone(), version.value.clone());
            }
            let local = una_project::ita::local_name(need);
            // Never mounted under a name that is not a local name, that is reserved or has the
            // skeleton of one, or that another need already mounts: a dependency's module must
            // never take the path of a standard module such as `std.host`, or of another
            // package's (rule 7). `ita.una` reports each as U7008 already.
            let unusable = !una_project::names::is_name_word(&local.value, 1, 64)
                || una_project::names::reserved_like(&local.value).is_some()
                || manifest
                    .mounts
                    .iter()
                    .any(|mount| mount.local == local.value);
            if unusable {
                located(
                    manifest_id,
                    vec![refuse(format!(
                        "`{}` cannot be the local name of a dependency",
                        local.value
                    ))],
                );
                continue;
            }
            manifest.mounts.push(una_project::Mount {
                local: local.value.clone(),
                src: format!("{}/src/", dependency.dir),
                edition: settings.edition,
            });
            let manifest_text = package.sources.file(manifest_id).text();
            manifest.boundaries.push(una_project::Boundary {
                module: una_project::Located {
                    value: local.value.clone(),
                    range: need.name.range.clone(),
                },
                ceiling: need
                    .may
                    .iter()
                    .map(|cap| una_project::Ceiling {
                        capability: cap.capability.clone(),
                        ops: cap.ops.iter().map(|op| op.value.clone()).collect(),
                        at: una_project::Located {
                            value: manifest_text
                                .get(cap.range.clone())
                                .unwrap_or("")
                                .to_owned(),
                            range: cap.range.clone(),
                        },
                    })
                    .collect(),
                package: true,
            });
            // A dependency's trusted modules (rule 53): each needs a trust line of the root by
            // identity and module path, and the lock's consent to the package's whole content.
            if !settings.trusted.is_empty() {
                let problems = trust_dependency(
                    dependency,
                    need,
                    &settings,
                    read.as_deref(),
                    lock_read.as_ref(),
                    &local.value,
                    manifest_text,
                    &mut manifest,
                );
                located(manifest_id, problems);
            }
        }
        // The root's own settings never reach into a dependency: a dependency's module is
        // trusted only by a trust line (ADR 0122 rule 53), and its boundaries, contracts and
        // entry are its own.
        let inside = |path: &str| {
            manifest.mounts.iter().any(|mount| {
                path.strip_prefix(mount.local.as_str())
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
            })
        };
        let mut reaching: Vec<ProjectError> = Vec::new();
        for entry in &manifest.trusted {
            if inside(&entry.value) && !manifest.dependency_trusted.contains(&entry.value) {
                reaching.push(ProjectError {
                    code: Code::U7008,
                    range: entry.range.clone(),
                    message: format!(
                        "`{}` is a dependency's module, which only a trust line of the root can trust",
                        entry.value
                    ),
                });
            }
        }
        for boundary in manifest.boundaries.iter().filter(|b| !b.package) {
            if inside(&boundary.module.value) {
                reaching.push(ProjectError {
                    code: Code::U7008,
                    range: boundary.module.range.clone(),
                    message: format!(
                        "`{}` is inside a dependency, whose package extent already bounds it",
                        boundary.module.value
                    ),
                });
            }
        }
        for contract in &manifest.contracts {
            if inside(&contract.function.value) {
                reaching.push(ProjectError {
                    code: Code::U7008,
                    range: contract.function.range.clone(),
                    message: format!(
                        "`{}` is a dependency's function, which the root's contracts cannot lock",
                        contract.function.value
                    ),
                });
            }
        }
        if let Some(entry) = &manifest.entry
            && inside(&entry.value)
        {
            reaching.push(ProjectError {
                code: Code::U7008,
                range: entry.range.clone(),
                message: format!(
                    "the run entry `{}` is a dependency's module; only the root is run",
                    entry.value
                ),
            });
        }
        located(manifest_id, reaching);
        match package.lock {
            Some(id) => {
                let (lock, errors) = parse_ita_lock(package.sources.file(id));
                let invalid = !errors.is_empty();
                located(id, errors);
                if !invalid && let Some(read) = read.as_deref() {
                    let consistency = check_lock_consistency(read, &lock, &versions);
                    located(manifest_id, consistency.in_manifest);
                    located(id, consistency.in_lock);
                    located(manifest_id, check_consent(read, &lock, &root_name));
                }
                // Each registry package: present, and verified as it was read (rule 17).
                let registry: Vec<ProjectError> = package
                    .registry
                    .iter()
                    .filter_map(|registered| {
                        let (code, message) = registered.problem.clone()?;
                        let range = lock.read.packages.get(registered.index)?.range.clone();
                        Some(ProjectError {
                            code,
                            range,
                            message,
                        })
                    })
                    .collect();
                located(id, registry);
            }
            None => located(
                manifest_id,
                vec![ProjectError {
                    code: Code::U7009,
                    range: 0..0,
                    message: "the package has no ita.lock; run `una ita lock`".to_owned(),
                }],
            ),
        }
        return (manifest, diagnostics);
    }
    let manifest = match package.manifest {
        Some(id) => {
            let (manifest, errors) = parse_manifest(package.sources.file(id).text());
            located(id, errors);
            manifest
        }
        // A single file has the default settings, except the edition of the package that
        // encloses it, when there is one (#1175).
        None => Manifest {
            edition: package.enclosing_edition.unwrap_or_default(),
            ..Manifest::default()
        },
    };
    match (package.lock, package.manifest) {
        (Some(id), _) => located(id, parse_lock(package.sources.file(id).text()).1),
        // A missing lock is located at the start of the package manifest.
        (None, Some(manifest)) => diagnostics.push(Diagnostic::error(
            Code::U7004,
            "the package has no verification.lock; add one next to una.toml",
            package.sources.location(Span::at(manifest, 0)),
        )),
        (None, None) => {}
    }
    (manifest, diagnostics)
}

/// Why a loaded dependency may not be used as the root writes it (ADR 0122 rules 42, 49 and 53,
/// and what this release does not load yet), each at the root's need. `local` is a path
/// dependency, whose name the root accepts with `accept name`.
pub(crate) fn dependency_problems(
    need: &ita::Need,
    dependency: &Manifest,
    root: &Manifest,
    local: bool,
) -> Vec<ProjectError> {
    let mut problems = Vec::new();
    let mut problem = |code: Code, message: String| {
        problems.push(ProjectError {
            code,
            range: need.range.clone(),
            message,
        });
    };
    let Some(read) = dependency.ita.as_deref() else {
        return problems;
    };
    if let Some(name) = &read.name
        && name.value != need.name.value
    {
        problem(
            Code::U7008,
            format!(
                "the package at this path is `{}`, not `{}`",
                name.value, need.name.value
            ),
        );
    }
    // Rule 42: a local package carries a name that a registry may hold (an unscoped name is
    // Itaúna's, and an owner may be any registry's, which no client can tell offline), so the
    // root accepts it by name.
    if local && need.accept_name.is_none() {
        problem(
            Code::U7012,
            format!(
                "`{}` is a local package with a name a registry may hold; write `accept name` under the need to use it",
                need.name.value
            ),
        );
    }
    // Rule 49: a dependency's edition is its dependent's or earlier.
    if dependency.edition > root.edition {
        problem(
            Code::U7012,
            format!(
                "`{}` is of edition {}, later than this package's {}",
                need.name.value,
                dependency.edition.name(),
                root.edition.name()
            ),
        );
    }
    let unsupported = [
        (!read.needs.is_empty(), "needs of its own"),
        (!read.labels.is_empty(), "label categories"),
        (!read.boundaries.is_empty(), "boundaries"),
        (!read.contracts.is_empty(), "contracts"),
    ];
    for (present, what) in unsupported {
        if present {
            problem(
                Code::U7008,
                format!(
                    "`{}` has {what}, which this release does not load for a dependency yet",
                    need.name.value
                ),
            );
        }
    }
    for cap in &need.may {
        if cap.capability.contains('.') {
            problem(
                Code::U7008,
                format!(
                    "the ceiling `{}` names a dependency's own capability, which this release does not load yet; a ceiling names host capabilities",
                    cap.capability
                ),
            );
        }
    }
    problems
}

/// The digest of a path dependency's content (ADR 0122 rule 28), its `ita.una` and its
/// modules, computed only from the bytes loaded, the same ones checked: no file is read again,
/// so content changed on disk after the load can never be what a consent is bound to, and
/// nothing stands in for a file that was not loaded.
pub(crate) fn dependency_digest(dependency: &Dependency) -> Result<String, String> {
    if dependency.manifest.is_none() || dependency.files.is_empty() {
        return Err(format!("`{}` was not loaded", dependency.dir));
    }
    let mut files = dependency.files.clone();
    una_project::content::digest_of(&mut files)
}

/// The trust line of the root that names the dependency of `need` at `version`: its name, its
/// major line when written, and no registry source, since a path package has none.
pub(crate) fn trust_line<'r>(
    root: &'r ita::Manifest,
    need: &ita::Need,
    version: Option<&str>,
) -> Option<&'r ita::TrustLine> {
    let line = version
        .and_then(|v| una_project::version::Version::parse_full(v).ok())
        .map(|v| v.line());
    root.trust.iter().find(|trust| {
        trust.name.value == need.name.value
            && trust.from.is_none()
            && trust.major.as_ref().is_none_or(|major| {
                una_project::version::Line::parse(&major.value) == line && line.is_some()
            })
    })
}

/// Rule 53 for one mounted dependency with trusted modules: every one of them covered by the
/// root's trust line, and the lock's `consented` digest the package's content now, with each
/// trusted module recorded. Then each is listed as trusted in the program, as an extent of its
/// own (rule 53.1). Until a crossing's ceiling depends on its direction, the package extent
/// and its trusted extents share one ceiling, the intersection of the root's `may` lines and
/// the trust line's: never wider than either. Problems are U7012 at the need.
#[allow(clippy::too_many_arguments)]
fn trust_dependency(
    dependency: &Dependency,
    need: &ita::Need,
    settings: &Manifest,
    root: Option<&ita::Manifest>,
    lock: Option<&una_project::ita::ItaLock>,
    local: &str,
    manifest_text: &str,
    manifest: &mut Manifest,
) -> Vec<ProjectError> {
    let refuse = |message: String| {
        vec![ProjectError {
            code: Code::U7012,
            range: need.range.clone(),
            message,
        }]
    };
    let version = settings.version.as_ref().map(|v| v.value.as_str());
    let Some(line) = root.and_then(|root| trust_line(root, need, version)) else {
        return refuse(format!(
            "`{}` has trusted modules, which no trust line of the root names; write `trust` with `{} modules ...` after reviewing them",
            need.name.value, need.name.value
        ));
    };
    let uncovered: Vec<&str> = settings
        .trusted
        .iter()
        .map(|t| t.value.as_str())
        .filter(|t| !line.modules.iter().any(|m| m.value == *t))
        .collect();
    if !uncovered.is_empty() {
        return refuse(format!(
            "the trust line of `{}` does not name its trusted module `{}`",
            need.name.value,
            uncovered.join("`, `")
        ));
    }
    let digest = match dependency_digest(dependency) {
        Ok(digest) => digest,
        Err(reason) => {
            return refuse(format!(
                "`{}` holds what a package may not: {reason}",
                need.name.value
            ));
        }
    };
    let consented = lock.and_then(|lock| {
        lock.read.packages.iter().find(|p| {
            p.name.value == need.name.value
                && matches!(
                    p.from.as_ref().map(|f| &f.value),
                    Some(ita::LockOrigin::Path(dir)) if dir.trim_end_matches('/') == dependency.dir
                )
        })
    });
    let consented_ok = consented.is_some_and(|p| {
        p.consented.as_ref().is_some_and(|c| c.value == digest)
            && settings
                .trusted
                .iter()
                .all(|t| p.trusted.iter().any(|m| m.value == t.value))
    });
    if !consented_ok {
        return refuse(format!(
            "the trust of `{}` is not consented for its content as it is now, {digest}; review its trusted modules and the code they run, then run `una ita lock --accept-trust {}@{digest}`",
            need.name.value, need.name.value
        ));
    }
    // One ceiling for the package and its trusted extents: the root's `may` ∩ the trust line's.
    let ceiling: Vec<una_project::Ceiling> = need
        .may
        .iter()
        .filter_map(|cap| {
            let ops: Vec<String> = cap
                .ops
                .iter()
                .map(|op| op.value.clone())
                .filter(|op| {
                    line.may.iter().any(|t| {
                        t.capability == cap.capability && t.ops.iter().any(|o| o.value == *op)
                    })
                })
                .collect();
            (!ops.is_empty()).then(|| una_project::Ceiling {
                capability: cap.capability.clone(),
                ops,
                at: una_project::Located {
                    value: manifest_text
                        .get(cap.range.clone())
                        .unwrap_or("")
                        .to_owned(),
                    range: cap.range.clone(),
                },
            })
        })
        .collect();
    if let Some(extent) = manifest
        .boundaries
        .iter_mut()
        .find(|b| b.package && b.module.value == local)
    {
        extent.ceiling.clone_from(&ceiling);
    }
    // Only the modules the dependency itself declares trusted: a trust line never makes
    // trusted a module its own package does not (rule 53).
    for module in line
        .modules
        .iter()
        .filter(|m| settings.trusted.iter().any(|t| t.value == m.value))
    {
        let path = format!("{local}.{}", module.value);
        manifest.trusted.push(una_project::Located {
            value: path.clone(),
            range: module.range.clone(),
        });
        manifest.boundaries.push(una_project::Boundary {
            module: una_project::Located {
                value: path.clone(),
                range: module.range.clone(),
            },
            ceiling: ceiling.clone(),
            package: true,
        });
        manifest.dependency_trusted.push(path);
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

    use super::*;

    /// A fresh package directory with the path dependency `deps/util`, whose module says `say`.
    fn with_dependency(name: &str, say: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("una-cli-package-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("deps/util/src")).unwrap();
        std::fs::write(
            dir.join("ita.una"),
            "ita demo\n  version 0.1.0\n  edition 0.4\n\nneeds\n  util from path \"deps/util\"\n    accept name\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("deps/util/ita.una"),
            "ita util\n  version 0.2.0\n  edition 0.4\n",
        )
        .unwrap();
        std::fs::write(dir.join("deps/util/src/say.una"), say).unwrap();
        std::fs::write(
            dir.join("src/demo.una"),
            "module demo\n\npub one() -> Int = 1\n",
        )
        .unwrap();
        dir
    }

    /// The digest of a path dependency is computed from the bytes loaded, never from its files
    /// read again: content B written after content A was loaded, as a swap between the load
    /// and the digest would be, leaves the digest A's, the digest of the code checked; and a
    /// dependency whose bytes were not kept has no digest at all.
    #[test]
    fn the_dependency_digest_is_of_the_bytes_loaded() {
        let a = "module say\n\npub one() -> Int = 1\n";
        let b = "module say\n\npub one() -> Int = 2\n";
        let dir = with_dependency("digest", a);
        let package = load(&dir).unwrap();
        let dependency = &package.dependencies[0];
        let digest_a = dependency_digest(dependency).unwrap();
        std::fs::write(dir.join("deps/util/src/say.una"), b).unwrap();
        assert_eq!(dependency_digest(dependency).unwrap(), digest_a);
        let reloaded = load(&dir).unwrap();
        let digest_b = dependency_digest(&reloaded.dependencies[0]).unwrap();
        assert_ne!(digest_a, digest_b);
        // The same as the digest of the bytes the module was checked from.
        let mut files = vec![
            una_project::content::File {
                path: "ita.una".to_owned(),
                bytes: b"ita util\n  version 0.2.0\n  edition 0.4\n".to_vec(),
            },
            una_project::content::File {
                path: "src/say.una".to_owned(),
                bytes: a.as_bytes().to_vec(),
            },
        ];
        assert_eq!(
            una_project::content::digest_of(&mut files).unwrap(),
            digest_a
        );
        let unkept = Dependency {
            need: dependency.need,
            dir: dependency.dir.clone(),
            manifest: dependency.manifest,
            modules: dependency.modules.clone(),
            error: None,
            files: Vec::new(),
            registry: None,
        };
        assert!(dependency_digest(&unkept).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
